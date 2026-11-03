use sherpa_onnx::{
	GenerationConfig,
	OfflineTts,
	OfflineTtsConfig,
	OfflineTtsVitsModelConfig,
	OfflineTtsModelConfig,
};
use tokio::sync::mpsc;
use tracing::error;

pub struct TtsService {
	// If you want to keep a cached instance for sample_rate()
	sample_rate: i32,
}

impl TtsService {
	pub fn new() -> Self {
		let config = Self::get_config();
		let tts = OfflineTts::create(&config).expect("Failed to create TTS engine");
		let sample_rate = tts.sample_rate();

		Self { sample_rate }
	}

	fn get_config() -> OfflineTtsConfig {
		OfflineTtsConfig {
			model: OfflineTtsModelConfig {
				vits: OfflineTtsVitsModelConfig {
					model: Some("./models/tts/vits-piper-vi_VN/model.onnx".into()),
					tokens: Some("./models/tts/vits-piper-vi_VN/tokens.txt".into()),
					data_dir: Some("./models/tts/vits-piper-vi_VN/espeak-ng-data".into()),
					noise_scale: 0.667,
					noise_scale_w: 0.8,
					length_scale: 1.0,
					..Default::default()
				},
				num_threads: 2,
				debug: false,
				..Default::default()
			},
			..Default::default()
		}
	}

	pub fn sample_rate(&self) -> i32 {
		self.sample_rate
	}

	pub fn generate_stream(
		&self,
		text: &str,
		sid: i32,
		speed: f32
	) -> mpsc::UnboundedReceiver<Result<Vec<f32>, String>> {
		let (tx, rx) = mpsc::unbounded_channel();

		let text = text.to_string();
		let gen_config = GenerationConfig {
			sid,
			speed,
			..Default::default()
		};

		std::thread::spawn(move || {
			let config = Self::get_config();
			let tts = match OfflineTts::create(&config) {
				Some(engine) => engine,
				None => {
					let _ = tx.send(Err(format!("Failed to create TTS engine.")));
					return;
				}
			};

			let result = tts.generate_with_config(
				&text,
				&gen_config,
				Some(move |samples: &[f32], _progress: f32| -> bool {
					if tx.send(Ok(samples.to_vec())).is_err() {
						return false; // Receiver dropped; stop generation.
					}
					true
				})
			);

			if result.is_none() {
				let _ = tx.send(Err("TTS generation failed".to_string()));
			}
		});

		rx
	}
}
