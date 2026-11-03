use axum::{
	extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
	response::IntoResponse,
};
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;

use crate::server::{base64_encode, AppState};
use crate::stt::session::{ClientCommand, RobotOut, Session};
use crate::tts::service::TtsService;

const SAMPLE_RATE: i32 = 16_000;

/// Explicit control messages shared by the app and ESP32. Audio remains PCM16
/// LE mono 16 kHz binary frames; the session's active state classifies it as
/// normal recording or an answer to a presentation question.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
	StartRecord,
	StopRecord,
	AnswerStart,
	AnswerStop,
	ContinuePresentation,
	SetMode { presentation: bool },
}

pub async fn stt_ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
	ws.on_upgrade(move |socket| handle_stt_socket(socket, state))
}

async fn handle_stt_socket(socket: WebSocket, state: AppState) {
	let (sink, mut stream) = socket.split();
	let (audio_tx, audio_rx) = mpsc::unbounded_channel::<Vec<f32>>();
	let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ClientCommand>();
	let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RobotOut>();
	let session = Session::new(&state.stt, SAMPLE_RATE, state.presentation.clone(), cmd_rx, audio_rx, out_tx);
	let session_task = tokio::spawn(session.run());

	let tts = state.tts.clone();
	let cmd_tx_writer = cmd_tx.clone();
	let writer = tokio::spawn(async move {
		let mut sink = sink;
		while let Some(out) = out_rx.recv().await {
			let ok = match out {
				RobotOut::Mic(enabled) => send(&mut sink, json!({"type":"mic","enabled":enabled})).await,
				RobotOut::Mode(mode) => send(&mut sink, json!({"type":"mode","mode":mode})).await,
				RobotOut::Transcript(text) => send(&mut sink, json!({"type":"transcript","text":text})).await,
				RobotOut::Error(message) => send(&mut sink, json!({"type":"error","message":message})).await,
				RobotOut::Score(score) => send(&mut sink, json!({"type":"score","result":serde_json::to_value(&score).unwrap_or(Value::Null)})).await,
				RobotOut::Speak(text) => speak(&mut sink, &tts, &text, &cmd_tx_writer).await,
			};
			if !ok { break; }
		}
	});

	while let Some(Ok(message)) = stream.next().await {
		match message {
			Message::Binary(bytes) => {
				let samples: Vec<f32> = bytes.chunks_exact(2)
					.map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0)
					.collect();
				let _ = audio_tx.send(samples);
			},
			Message::Text(text) => match serde_json::from_str::<ClientMsg>(&text) {
				Ok(ClientMsg::StartRecord) => { let _ = cmd_tx.send(ClientCommand::StartRecord); },
				Ok(ClientMsg::StopRecord) => { let _ = cmd_tx.send(ClientCommand::StopRecord); },
				Ok(ClientMsg::AnswerStart) => { let _ = cmd_tx.send(ClientCommand::AnswerStart); },
				Ok(ClientMsg::AnswerStop) => { let _ = cmd_tx.send(ClientCommand::AnswerStop); },
				Ok(ClientMsg::ContinuePresentation) => { let _ = cmd_tx.send(ClientCommand::ContinuePresentation); },
				Ok(ClientMsg::SetMode { presentation }) => { state.presentation.store(presentation, Ordering::Relaxed); },
				Err(error) => log::warn!("Ignoring invalid WebSocket control message: {error}"),
			},
			Message::Close(_) => break,
			_ => {},
		}
	}
	 session_task.abort();
	writer.abort();
}

async fn send(sink: &mut SplitSink<WebSocket, Message>, value: Value) -> bool {
	sink.send(Message::Text(value.to_string().into())).await.is_ok()
}

async fn speak(
	sink: &mut SplitSink<WebSocket, Message>,
	tts: &TtsService,
	text: &str,
	cmd_tx: &mpsc::UnboundedSender<ClientCommand>,
) -> bool {
	let sample_rate = tts.sample_rate();
	let mut rx = tts.generate_stream(text, 0, 1.0);
	while let Some(chunk) = rx.recv().await {
		match chunk {
			Ok(samples) => {
				let bytes: Vec<u8> = samples.iter().flat_map(|sample| sample.to_le_bytes()).collect();
				if !send(sink, json!({"type":"audio","samples":base64_encode(&bytes),"sample_rate":sample_rate,"is_final":false})).await {
					return false;
				}
			},
			Err(error) => {
				if !send(sink, json!({"type":"error","message":error})).await { return false; }
			},
		}
	}
	if !send(sink, json!({"type":"done"})).await { return false; }
	// Let playback drain before the session reopens the microphone.
	let playback_seconds = 0.0f32;
	let _ = playback_seconds; // Playback completion can be reported by the ESP32 in a follow-up protocol revision.
	let _ = cmd_tx.send(ClientCommand::SpeechDone);
	true
}
