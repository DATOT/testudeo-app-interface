use reqwest;

use crate::llm::types::{ MessageResponse, ScoreResponse };

#[tauri::command]
pub async fn process_message(message: String) -> Result<String, String> {
	let client = reqwest::Client::new();

	let res = client
		.post("https://testudeo-ai.lamtiendat520.workers.dev/message")
		.header("Content-Type", "text/plain")
		.body(message)
		.send().await
		.map_err(|e| e.to_string())?;

	let parsed: MessageResponse = res.json().await.map_err(|e| e.to_string())?;

	Ok(parsed.content)
}

#[tauri::command]
pub async fn score_presentation(conversation: String) -> Result<ScoreResponse, String> {
	let res = reqwest::Client
		::new()
		.post("https://testudeo-ai.lamtiendat520.workers.dev/score")
		.header("Content-Type", "text/plain")
		.body(conversation)
		.send().await
		.map_err(|e| e.to_string())?
		.error_for_status()
		.map_err(|e| e.to_string())?;

	res.json::<ScoreResponse>().await.map_err(|e| e.to_string())
}
