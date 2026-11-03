use axum::{ Router, extract::FromRef, routing::get };
use log::info;
use std::sync::{ Arc, atomic::AtomicBool };
use tokio::net::TcpListener; // <- was missing, server.rs doesn't compile without it
use crate::{ stt::{ service::SttService, ws::stt_ws_handler }, tts::service::TtsService };

#[derive(Clone)]
pub struct AppState {
	pub tts: Arc<TtsService>,
	pub stt: Arc<SttService>,
	pub presentation: Arc<AtomicBool>, // set by the control interface
}

// keeps your existing tts_ws_handler(State<Arc<TtsService>>) unchanged
impl FromRef<AppState> for Arc<TtsService> {
	fn from_ref(s: &AppState) -> Self {
		s.tts.clone()
	}
}

pub(crate) fn base64_encode(data: &[u8]) -> String {/* same body, now pub(crate) */}

pub async fn start_server(addr: &str, presentation: Arc<AtomicBool>) {
	let state = AppState {
		tts: Arc::new(TtsService::new()),
		stt: Arc::new(SttService::new()),
		presentation,
	};

	let app = Router::new()
		.route("/ws/tts", get(tts_ws_handler))
		.route("/ws/stt", get(stt_ws_handler))
		.with_state(state);

	let listener = TcpListener::bind(addr).await.unwrap();
	info!("Server listening on {}", addr);
	axum::serve(listener, app).await.unwrap();
}
