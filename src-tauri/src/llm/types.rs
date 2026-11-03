use serde::{ Deserialize, Serialize };

#[derive(Deserialize)]
pub struct MessageResponse {
	pub content: String,
}
#[derive(Deserialize, Serialize, Clone)]
pub struct ScoreResponse {
	pub knowledge_score: serde_json::Value,
	pub feynman_score: serde_json::Value,
	pub qa_score: serde_json::Value,
	pub feedback: String,
	pub suggestion: String,
}
