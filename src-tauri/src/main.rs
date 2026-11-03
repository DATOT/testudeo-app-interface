// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod stt;
mod server;
mod tts;
mod ble;
mod llm;

use std::sync::Arc;
use tauri::{ AppHandle, Emitter };

fn main() {
	std::thread::spawn(move || {
		let rt = tokio::runtime::Runtime::new().unwrap();
		rt.block_on(async {
			server::start_websocket_server("0.0.0.0:3000").await;
		});
	});
	app_lib::run();
}
