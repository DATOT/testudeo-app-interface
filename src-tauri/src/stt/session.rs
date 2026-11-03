use std::collections::VecDeque;
use std::sync::{ atomic::{ AtomicBool, Ordering }, Arc };
use tokio::sync::mpsc;
use uuid::Uuid;

// ADJUST these paths to your module layout.
use crate::llm::chunker::utils::{ embed_text, query_search_tree, reset_previous_context, search_presentation };
use crate::llm::client::{ process_message, score_presentation };
use crate::llm::types::ScoreResponse;
use crate::stt::service::{ SttEvent, SttInput, SttService };
use crate::stt::types::{ Conversation, Dialogue, Role, Task };

const SILENCE_SECS: f32 = 3.0;
const CONTEXT_WORDS: usize = 24;
const QUESTION_CHANCE_PERCENT: u128 = 20;
const RELATION_THRESHOLD: f32 = 0.55; // cosine; tune for BGE-M3
const MIN_WORDS_FOR_SEARCH: usize = 4; // ignore "ừ", "à", ...

/// From the websocket (robot button / playback finished).
pub enum ClientCommand {
	Button,
	SpeechDone,
}

/// To the websocket (and from there to the ESP32 / UI).
pub enum RobotOut {
	/// Tell the ESP32 to start/stop capturing + sending audio.
	Mic(bool),
	/// Synthesize this text and stream it to the robot.
	Speak(String),
	Transcript(String),
	Score(ScoreResponse),
	Error(String),
}

#[derive(PartialEq, Clone, Copy)]
enum Mode {
	Normal,
	Presentation,
}

#[derive(PartialEq, Clone, Copy)]
enum State {
	Idle,
	Recording,
	/// Mic muted: waiting for the LLM, or the robot is speaking.
	Busy,
}

pub struct Session {
	cmd_rx: mpsc::UnboundedReceiver<ClientCommand>,
	audio_rx: mpsc::UnboundedReceiver<Vec<f32>>,
	out_tx: mpsc::UnboundedSender<RobotOut>,
	stt_tx: mpsc::UnboundedSender<SttInput>,
	stt_rx: mpsc::UnboundedReceiver<SttEvent>,
	presentation_flag: Arc<AtomicBool>,

	mode: Mode,
	state: State,
	flushing: bool, // Finish sent, waiting for Flushed
	awaiting_speech: bool,
	resume_after_speech: bool,
	stop_after_speech: bool, // button pressed while the robot was busy

	conversation: Conversation,
	window: VecDeque<String>, // rolling 24-word context
	turn_text: String, // normal mode: what was said this turn
	last_chunks: Vec<String>, // matched chunks of the last presentation search
}

impl Session {
	pub fn new(
		stt: &SttService,
		sample_rate: i32,
		presentation_flag: Arc<AtomicBool>,
		cmd_rx: mpsc::UnboundedReceiver<ClientCommand>,
		audio_rx: mpsc::UnboundedReceiver<Vec<f32>>,
		out_tx: mpsc::UnboundedSender<RobotOut>
	) -> Self {
		let (stt_tx, stt_rx) = stt.create_stream_session(sample_rate, SILENCE_SECS);
		Self {
			cmd_rx,
			audio_rx,
			out_tx,
			stt_tx,
			stt_rx,
			presentation_flag,
			mode: Mode::Normal,
			state: State::Idle,
			flushing: false,
			awaiting_speech: false,
			resume_after_speech: false,
			stop_after_speech: false,
			conversation: Conversation(Vec::new()),
			window: VecDeque::new(),
			turn_text: String::new(),
			last_chunks: Vec::new(),
		}
	}

	pub async fn run(mut self) {
		loop {
			tokio::select! {
				cmd = self.cmd_rx.recv() => match cmd {
					Some(c) => self.on_command(c).await,
					None => break,
				},
				audio = self.audio_rx.recv() => match audio {
					// Muted / idle audio is simply dropped.
					Some(samples) => if self.state == State::Recording {
						let _ = self.stt_tx.send(SttInput::Audio(samples));
					},
					None => break,
				},
				Some(ev) = self.stt_rx.recv() => self.on_stt_event(ev).await,
			}
		}
	}

	fn out(&self, o: RobotOut) {
		let _ = self.out_tx.send(o);
	}

	// ---------------- commands ----------------

	async fn on_command(&mut self, cmd: ClientCommand) {
		match cmd {
			ClientCommand::Button => match self.state {
				State::Idle => self.start(),
				State::Recording => self.stop(),
				State::Busy => {
					if self.mode == Mode::Presentation && !self.flushing {
						self.stop_after_speech = true; // finish once the robot is done
					}
				}
			},
			ClientCommand::SpeechDone => {
				if self.awaiting_speech {
					self.awaiting_speech = false;
					if self.resume_after_speech {
						if self.stop_after_speech {
							self.finish_presentation().await;
						} else {
							self.resume();
						}
					} else {
						self.state = State::Idle;
					}
				}
			}
		}
	}

	fn start(&mut self) {
		// Mode is latched here so a mode switch never happens mid-recording.
		let mode = if self.presentation_flag.load(Ordering::Relaxed) {
			Mode::Presentation
		} else {
			Mode::Normal
		};
		if mode != self.mode || mode == Mode::Presentation {
			self.conversation = Conversation(Vec::new());
		}
		self.mode = mode;
		self.window.clear();
		self.turn_text.clear();
		self.last_chunks.clear();
		self.stop_after_speech = false;
		reset_previous_context();
		self.resume();
	}

	fn resume(&mut self) {
		// Throw away events produced before the mute, then reset the recognizer.
		while self.stt_rx.try_recv().is_ok() {}
		let _ = self.stt_tx.send(SttInput::Reset);
		self.state = State::Recording;
		self.out(RobotOut::Mic(true));
	}

	fn stop(&mut self) {
		self.state = State::Busy;
		self.flushing = true;
		self.out(RobotOut::Mic(false));
		let _ = self.stt_tx.send(SttInput::Finish);
	}

	// ---------------- STT events ----------------

	async fn on_stt_event(&mut self, ev: SttEvent) {
		match ev {
			SttEvent::Final(text) if self.state == State::Recording || self.flushing => {
				self.on_final(text).await;
			}
			SttEvent::Silence if self.state == State::Recording && self.mode == Mode::Presentation => {
				self.on_silence().await;
			}
			SttEvent::Flushed if self.flushing => {
				self.flushing = false;
				match self.mode {
					Mode::Normal => self.normal_reply().await,
					Mode::Presentation => self.finish_presentation().await,
				}
			}
			SttEvent::Error(e) => self.out(RobotOut::Error(e)),
			_ => {}
		}
	}

	async fn on_final(&mut self, text: String) {
		self.out(RobotOut::Transcript(text.clone()));
		self.conversation.0.push(Dialogue::new(Role::User, text.clone(), Task::None));
		self.push_words(&text);
		self.turn_text.push_str(&text);
		self.turn_text.push(' ');

		if self.mode == Mode::Presentation && self.state == State::Recording {
			self.track_topic(text).await;
		}
	}

	// ---------------- presentation mode ----------------

	/// Keep the "previous context" fresh and detect topic shifts.
	async fn track_topic(&mut self, text: String) {
		if text.split_whitespace().count() < MIN_WORDS_FOR_SEARCH {
			return;
		}

		let query = text.clone();
		let result = tokio::task
			::spawn_blocking(move || {
				let v = embed_text(&query).map_err(|e| e.to_string())?;
				search_presentation(&v, 3, 64, 1, RELATION_THRESHOLD)
			}).await
			.map_err(|e| e.to_string())
			.and_then(|r| r);

		let hit = match result {
			Ok(h) => h,
			Err(e) => {
				self.out(RobotOut::Error(e));
				return;
			}
		};

		self.last_chunks = hit.chunks.iter().map(|c| c.text.clone()).collect();

		if hit.topic_shift && roll(QUESTION_CHANCE_PERCENT) {
			// Ask about the section they just LEFT (previous_chunks was read
			// before the search overwrote the context).
			let prompt = format!(
				"Người dùng vừa chuyển sang chủ đề khác trong bài thuyết trình.\n\
				 Phần họ vừa trình bày trước đó (từ tài liệu):\n{}\n\
				 Họ vừa nói: \"{}\"\n\
				 Hãy đặt MỘT câu hỏi ngắn về phần vừa trình bày để kiểm tra mức độ hiểu bài.",
				format_chunks(&hit.previous_chunks),
				self.window_text()
			);
			self.begin_busy();
			self.reply(prompt, Task::AskQuestion, true).await;
		}
	}

	async fn on_silence(&mut self) {
		self.conversation.0.push(
			Dialogue::new(Role::VAD, format!("silence {}s", SILENCE_SECS as i32), Task::None)
		);
		let prompt = format!(
			"Người dùng đang thuyết trình và im lặng khoảng {} giây. Họ vừa nói: \"{}\"\n\
			 Nội dung tài liệu liên quan:\n{}\n\
			 Hãy đưa ra MỘT gợi ý rất ngắn (1 câu) giúp họ nói tiếp, không nói thay họ.",
			SILENCE_SECS as i32,
			self.window_text(),
			format_chunks(&self.last_chunks)
		);
		self.begin_busy();
		self.reply(prompt, Task::GiveHint, true).await;
	}

	async fn finish_presentation(&mut self) {
		self.state = State::Busy;
		self.stop_after_speech = false;
		self.out(RobotOut::Mic(false));

		if !self.conversation.is_empty() {
			match score_presentation(self.conversation.to_string()).await {
				Ok(score) => self.out(RobotOut::Score(score)),
				Err(e) => self.out(RobotOut::Error(e)),
			}
		}
		self.state = State::Idle;
	}

	// ---------------- normal mode ----------------

	async fn normal_reply(&mut self) {
		let query = std::mem::take(&mut self.turn_text).trim().to_string();
		if query.is_empty() {
			self.state = State::Idle;
			return;
		}

		let q = query.clone();
		let chunks: Vec<String> = tokio::task
			::spawn_blocking(move || {
				let v = embed_text(&q).map_err(|e| e.to_string())?;
				query_search_tree(&v, 3, 64, 1)
			}).await
			.ok()
			.and_then(|r| r.ok())
			.map(|cs| cs.into_iter().map(|c| c.text).collect())
			.unwrap_or_default();

		let prompt = format!(
			"Tài liệu liên quan:\n{}\n\nHội thoại:\n{}\nHãy trả lời lượt nói cuối của User.",
			format_chunks(&chunks),
			self.conversation
		);
		self.reply(prompt, Task::None, false).await;
	}

	// ---------------- shared ----------------

	/// Mute the mic while we think + speak.
	fn begin_busy(&mut self) {
		self.state = State::Busy;
		self.out(RobotOut::Mic(false));
	}

	/// Ask the LLM, store the answer in the conversation, hand it to TTS.
	/// `resume` = presentation mode (re-open the mic afterwards).
	async fn reply(&mut self, prompt: String, task: Task, resume: bool) {
		match process_message(prompt).await {
			Ok(text) if !text.trim().is_empty() => {
				self.conversation.0.push(Dialogue::new(Role::AI, text.clone(), task));
				self.awaiting_speech = true;
				self.resume_after_speech = resume;
				self.out(RobotOut::Speak(text));
			}
			other => {
				if let Err(e) = other {
					self.out(RobotOut::Error(e));
				}
				// Nothing to say: behave as if speech finished immediately.
				self.awaiting_speech = true;
				self.resume_after_speech = resume;
				self.on_command(ClientCommand::SpeechDone).await;
			}
		}
	}

	fn push_words(&mut self, text: &str) {
		for w in text.split_whitespace() {
			if self.window.len() >= CONTEXT_WORDS {
				self.window.pop_front();
			}
			self.window.push_back(w.to_string());
		}
	}

	fn window_text(&self) -> String {
		self.window.iter().map(String::as_str).collect::<Vec<_>>().join(" ")
	}
}

fn format_chunks(chunks: &[String]) -> String {
	if chunks.is_empty() {
		return "(không có)".to_string();
	}
	chunks
		.iter()
		.take(6)
		.map(|c| format!("- {}", c.trim().chars().take(300).collect::<String>()))
		.collect::<Vec<_>>()
		.join("\n")
}

/// true with `percent`% probability (uuid is already a dependency; swap for `rand` if you like).
fn roll(percent: u128) -> bool {
	Uuid::new_v4().as_u128() % 100 < percent
}
