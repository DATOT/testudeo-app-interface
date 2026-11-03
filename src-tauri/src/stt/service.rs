use sherpa_onnx::{
	OnlineRecognizer,
	OnlineRecognizerConfig,
	OnlineStream,
	VoiceActivityDetector,
	VadModelConfig,
};
use tokio::sync::mpsc;

/// Commands sent INTO the recognizer thread.
pub enum SttInput {
	/// 16 kHz mono f32 samples from the robot mic.
	Audio(Vec<f32>),
	/// Drop all state (call when the mic is re-enabled after the robot spoke).
	Reset,
	/// The user stopped recording: flush the recognizer, emit the last text, then `Flushed`.
	Finish,
}

/// Events coming OUT of the recognizer thread.
pub enum SttEvent {
	/// A finished sentence (endpoint reached, or flushed by silence / Finish).
	Final(String),
	/// The user spoke, then VAD saw `silence_secs` of no speech.
	Silence,
	/// Answer to `SttInput::Finish`; every `Final` for that recording came before it.
	Flushed,
	Error(String),
}

pub struct SttService;

impl SttService {
	pub fn new() -> Self {
		Self
	}

	fn get_config() -> OnlineRecognizerConfig {
		let mut config = OnlineRecognizerConfig::default();

		config.model_config.transducer.encoder = Some(
			"./models/stt/sherpa-onnx-zipformer-vi-int8/encoder.onnx".into()
		);
		config.model_config.transducer.decoder = Some(
			"./models/stt/sherpa-onnx-zipformer-vi-int8/decoder.onnx".into()
		);
		config.model_config.transducer.joiner = Some(
			"./models/stt/sherpa-onnx-zipformer-vi-int8/joiner.onnx".into()
		);
		config.model_config.tokens = Some(
			"./models/stt/sherpa-onnx-zipformer-vi-int8/tokens.txt".into()
		);
		config.model_config.num_threads = 2;
		config.model_config.debug = false;

		// IMPORTANT: without this `is_endpoint()` never fires.
		config.enable_endpoint = true;
		// Tune if sentences are cut too early/late (field names: rule1/2/3_min_*):
		// config.rule2_min_trailing_silence = 1.2;

		config
	}

	pub fn create_stream_session(
		&self,
		sample_rate: i32,
		silence_secs: f32
	) -> (mpsc::UnboundedSender<SttInput>, mpsc::UnboundedReceiver<SttEvent>) {
		let (in_tx, mut in_rx) = mpsc::unbounded_channel::<SttInput>();
		let (ev_tx, ev_rx) = mpsc::unbounded_channel::<SttEvent>();

		std::thread::spawn(move || {
			let recognizer = match OnlineRecognizer::create(&Self::get_config()) {
				Some(r) => r,
				None => {
					let _ = ev_tx.send(SttEvent::Error("Failed to create OnlineRecognizer".into()));
					return;
				}
			};

			let mut vad_config = VadModelConfig::default();
			vad_config.silero_vad.model = Some("./models/stt/silero_vad.onnx".to_string());
			vad_config.sample_rate = sample_rate;
			vad_config.num_threads = 1;

			let vad = match VoiceActivityDetector::create(&vad_config, 30.0) {
				Some(v) => v,
				None => {
					let _ = ev_tx.send(SttEvent::Error("Failed to create VoiceActivityDetector".into()));
					return;
				}
			};

			let mut stream = recognizer.create_stream();

			// Silence is counted in SAMPLES, not wall-clock, so bursty network
			// delivery from the ESP32 can't fake or hide a pause.
			let silence_limit = (sample_rate as f32 * silence_secs) as usize;
			let mut silent_samples = 0usize;
			let mut heard_speech = false; // spoke since the last hint?

			while let Some(input) = in_rx.blocking_recv() {
				match input {
					SttInput::Reset => {
						stream = recognizer.create_stream();
						vad.reset();
						silent_samples = 0;
						heard_speech = false;
					}

					SttInput::Finish => {
						stream.input_finished();
						decode_ready(&recognizer, &stream);
						let text = current_text(&recognizer, &stream);
						if !text.is_empty() {
							let _ = ev_tx.send(SttEvent::Final(text));
						}
						stream = recognizer.create_stream();
						vad.reset();
						silent_samples = 0;
						heard_speech = false;
						if ev_tx.send(SttEvent::Flushed).is_err() {
							break;
						}
					}

					SttInput::Audio(samples) => {
						// VAD is only used as a speech/no-speech detector here.
						vad.accept_waveform(&samples);
						let speaking = vad.detected();
						while !vad.is_empty() {
							vad.pop(); // discard segments so the buffer never fills
						}

						if speaking {
							heard_speech = true;
							silent_samples = 0;
						} else {
							silent_samples += samples.len();
						}

						stream.accept_waveform(sample_rate, &samples);
						decode_ready(&recognizer, &stream);
						let mut text = current_text(&recognizer, &stream);

						if recognizer.is_endpoint(&stream) {
							if !text.is_empty() && ev_tx.send(SttEvent::Final(text.clone())).is_err() {
								break;
							}
							recognizer.reset(&stream);
							text.clear();
						}

						if heard_speech && silent_samples >= silence_limit {
							// Don't lose a half-finished sentence.
							if !text.is_empty() {
								let _ = ev_tx.send(SttEvent::Final(text));
								recognizer.reset(&stream);
							}
							if ev_tx.send(SttEvent::Silence).is_err() {
								break;
							}
							heard_speech = false;
							silent_samples = 0;
						}
					}
				}
			}
		});

		(in_tx, ev_rx)
	}
}

fn decode_ready(recognizer: &OnlineRecognizer, stream: &OnlineStream) {
	while recognizer.is_ready(stream) {
		recognizer.decode(stream);
	}
}

fn current_text(recognizer: &OnlineRecognizer, stream: &OnlineStream) -> String {
	recognizer
		.get_result(stream)
		.map(|r| r.text.trim().to_string())
		.unwrap_or_default()
}
