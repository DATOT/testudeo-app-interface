#[derive(Debug, Clone)]
pub struct SemanticChunk {
	/// UUID (v4) string.
	pub id: String,
	/// Id of the chunk one layer up that contains this chunk.
	/// For the broadest layer (sections) this is the document id (also a UUID).
	pub parent_id: Option<String>,
	pub text: String,
	pub span: (usize, usize),
	pub embedding: Vec<f32>,
}
