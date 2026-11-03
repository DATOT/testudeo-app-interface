use axum::{
	extract::{ ws::{ Message, WebSocket, WebSocketUpgrade }, State },
	response::IntoResponse,
};
use futures_util::{ stream::SplitSink, SinkExt, StreamExt };
use serde::Deserialize;
use serde_json::{ json, Value };
use std::{ sync::atomic::Ordering, time::{ Duration, Instant } };
use tokio::sync::mpsc;

use crate::server::{ base64_encode, AppState };
use crate::stt::session::{ ClientCommand, RobotOut, Session };
use crate::tts::service::TtsService;

const SAMPLE_RATE: i32 = 16_000;

/// Text frames from the robot / control UI.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
	/// Physical record button.
	Button,
	/// Control interface: switch normal / presentation (applies on the next recording).
	SetMode {
		presentation: bool,
	},
}

pub async fn stt_ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
	ws.on_upgrade(move |socket| handle_stt_socket(socket, state))
}

/// Protocol
///   robot -> server : BINARY = PCM16 LE mono 16 kHz mic audio
///                     TEXT   = {"type":"button"} | {"type":"set_mode","presentation":true}
///   server -> robot : {"type":"mic","enabled":bool}   start/stop capturing audio
///                     {"type":"audio","samples":b64 f32 LE,"sample_rate":n,"is_final":false}
///                     {"type":"done"}  (all audio for one reply sent)
///                     {"type":"transcript","text":..} {"type":"score","result":{..}} {"type":"error",..}
async fn handle_stt_socket(socket: WebSocket, state: AppState) {
	let (sink, mut stream) = socket.split();

	let (audio_tx, audio_rx) = mpsc::unbounded_channel::<Vec<f32>>();
	let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ClientCommand>();
	let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RobotOut>();

	let session = Session::new(&state.stt, SAMPLE_RATE, state.presentation.clone(), cmd_rx, audio_rx, out_tx);
	let session_task = tokio::spawn(session.run());

	// Writer: session output -> websocket (also drives TTS playback).
	let tts = state.tts.clone();
	let cmd_tx_w = cmd_tx.clone();
	let writer = tokio::spawn(async move {
		let mut sink = sink;
		while let Some(out) = out_rx.recv().await {
			let ok = match out {
				RobotOut::Mic(on) => send(&mut sink, json!({"type":"mic","enabled":on})).await,
				RobotOut::Transcript(t) => send(&mut sink, json!({"type":"transcript","text":t})).await,
				RobotOut::Error(e) => send(&mut sink, json!({"type":"error","message":e})).await,
				RobotOut::Score(s) =>
					send(&mut sink, json!({"type":"score","result":serde_json::to_value(&s).unwrap_or(Value::Null)})).await,
				RobotOut::Speak(text) => speak(&mut sink, &tts, &text, &cmd_tx_w).await,
			};
			if !ok {
				break;
			}
		}
	});

	// Reader: websocket -> session.
	while let Some(Ok(msg)) = stream.next().await {
		match msg {
			Message::Binary(bytes) => {
				let samples: Vec<f32> = bytes
					.chunks_exact(2)
					.map(|c| (i16::from_le_bytes([c[0], c[1]]) as f32) / 32768.0)
					.collect();
				let _ = audio_tx.send(samples);
			}
			Message::Text(t) => match serde_json::from_str::<ClientMsg>(&t) {
				Ok(ClientMsg::Button) => {
					let _ = cmd_tx.send(ClientCommand::Button);
				}
				Ok(ClientMsg::SetMode { presentation }) => {
					state.presentation.store(presentation, Ordering::Relaxed);
				}
				Err(_) => {}
			},
			Message::Close(_) => break,
			_ => {}
		}
	}

	session_task.abort();
	writer.abort();
}

async fn send(sink: &mut SplitSink<WebSocket, Message>, v: Value) -> bool {
	sink.send(Message::Text(v.to_string().into())).await.is_ok()
}

/// Stream TTS audio to the robot, wait for it to finish playing, then tell the
/// session so it can re-open the mic.
async fn speak(
	sink: &mut SplitSink<WebSocket, Message>,
	tts: &TtsService,
	text: &str,
	cmd_tx: &mpsc::UnboundedSender<ClientCommand>
) -> bool {
	let sample_rate = tts.sample_rate();
	let started = Instant::now();
	let mut total_samples = 0usize;

	let mut rx = tts.generate_stream(text, 0, 1.0);
	while let Some(chunk) = rx.recv().await {
		let ok = match chunk {
			Ok(samples) => {
				total_samples += samples.len();
				let bytes: Vec<u8> = samples
					.iter()
					.flat_map(|s| s.to_le_bytes())
					.collect();
				send(
					sink,
					json!({"type":"audio","samples":base64_encode(&bytes),"sample_rate":sample_rate,"is_final":false})
				).await
			}
			Err(e) => send(sink, json!({"type":"error","message":e})).await,
		};
		if !ok {
			return false;
		}
	}
	if !send(sink, json!({"type":"done"})).await {
		return false;
	}

	// Audio is generated faster than real time: wait out the remaining playback
	// (+ small margin) so the mic doesn't hear the robot's own voice.
	// Better: have the ESP32 send {"type":"playback_done"} and use that instead.
	let playback = Duration::from_secs_f32((total_samples as f32) / (sample_rate as f32));
	let wait = (playback + Duration::from_millis(400)).saturating_sub(started.elapsed());
	tokio::time::sleep(wait).await;

	let _ = cmd_tx.send(ClientCommand::SpeechDone);
	true
}
