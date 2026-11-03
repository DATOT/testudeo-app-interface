use crate::llm::chunker::types::SemanticChunk;
use crate::llm::chunker::model::{ with_embedding_model, with_search_tree };
use std::error::Error;
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;

pub struct ContextHit {
	pub chunks: Vec<SemanticChunk>,
	pub related_level: Option<usize>,
	pub topic_shift: bool,
	/// Text of the previous context (layer 1 = sentence groups), read before the search.
	pub previous_chunks: Vec<String>,
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
	debug_assert_eq!(a.len(), b.len(), "embedding dimensions must match");

	let dot: f32 = a
		.iter()
		.zip(b.iter())
		.map(|(x, y)| x * y)
		.sum();
	let norm_a = a
		.iter()
		.map(|x| x * x)
		.sum::<f32>()
		.sqrt();
	let norm_b = b
		.iter()
		.map(|x| x * x)
		.sum::<f32>()
		.sqrt();

	if norm_a == 0.0 || norm_b == 0.0 {
		0.0
	} else {
		dot / (norm_a * norm_b)
	}
}

#[allow(dead_code)]
fn normalize(vec: &mut [f32]) {
	let norm = vec
		.iter()
		.map(|x| x * x)
		.sum::<f32>()
		.sqrt();
	if norm > 0.0 {
		for v in vec.iter_mut() {
			*v /= norm;
		}
	}
}

pub fn embed_texts<S: AsRef<str>>(texts: &[S]) -> Result<Vec<Vec<f32>>, Box<dyn Error>> {
	let inputs: Vec<&str> = texts
		.iter()
		.map(|t| t.as_ref())
		.collect();

	with_embedding_model(|model| model.embed(inputs, None)).map_err(|e| e.into())
}

pub fn embed_text<S: AsRef<str>>(text: S) -> Result<Vec<f32>, Box<dyn Error>> {
	let mut res = embed_texts(&[text])?;
	res.pop().ok_or_else(|| "Failed to generate embedding".into())
}

pub fn chunk_text(text: &str) -> Result<Vec<SemanticChunk>, Box<dyn Error>> {
	let sentence_tuples: Vec<(usize, &str)> = text.split_sentence_bound_indices().collect();

	if sentence_tuples.is_empty() {
		return Ok(Vec::new());
	}

	let texts: Vec<String> = sentence_tuples
		.iter()
		.map(|(_, s)| s.to_string())
		.collect();

	let embeddings = embed_texts(&texts)?;
	let mut chunks = Vec::with_capacity(sentence_tuples.len());

	for ((start, s), embedding) in sentence_tuples.into_iter().zip(embeddings) {
		let end = start + s.len();

		chunks.push(SemanticChunk {
			id: Uuid::new_v4().to_string(),
			parent_id: None,
			text: s.to_string(),
			span: (start, end),
			embedding,
		});
	}

	Ok(chunks)
}

/// Merges neighbouring chunks whose similarity is >= threshold.
/// Every input chunk gets its `parent_id` set to the id of the combined
/// chunk it ended up in.
pub fn combine_chunks(
	initial_chunks: &mut [SemanticChunk],
	threshold: f32
) -> Result<Vec<SemanticChunk>, Box<dyn Error>> {
	if initial_chunks.is_empty() {
		return Ok(Vec::new());
	}

	let mut combined: Vec<SemanticChunk> = Vec::new();

	for chunk in initial_chunks.iter_mut() {
		if let Some(last) = combined.last_mut() {
			let similarity = cosine_similarity(&last.embedding, &chunk.embedding);

			if similarity >= threshold {
				chunk.parent_id = Some(last.id.clone());

				last.text.push_str(&chunk.text);
				last.span.1 = chunk.span.1;
				last.embedding = embed_text(&last.text)?;
				continue;
			}
		}

		// Start a new chunk
		let id = Uuid::new_v4().to_string();
		chunk.parent_id = Some(id.clone());

		combined.push(SemanticChunk {
			id,
			parent_id: None,
			text: chunk.text.clone(),
			span: chunk.span,
			embedding: chunk.embedding.clone(),
		});
	}

	Ok(combined)
}

// Database stub, to be implemented.
pub fn store_chunks(_layer: usize, _chunks: &[SemanticChunk]) {}

/// Replace the contents of a search tree layer.
pub fn populate_search_tree_layer(
	layer_idx: usize,
	chunks: &[SemanticChunk]
) -> Result<(), String> {
	let chunks = chunks.to_vec();
	with_search_tree(|search_tree| search_tree.populate_layer(layer_idx, chunks))
}

/// Add chunks to a search tree layer, keeping other documents.
pub fn append_search_tree_layer(layer_idx: usize, chunks: &[SemanticChunk]) -> Result<(), String> {
	let chunks = chunks.to_vec();
	with_search_tree(|search_tree| search_tree.append_layer(layer_idx, chunks))
}

pub struct ContextSearchResult {
	pub chunks: Vec<SemanticChunk>,
	/// See `ContextSearch::related_level`.
	pub related_level: Option<usize>,
}

pub fn query_search_tree(
	query_vector: &[f32],
	top_parent_k: usize,
	ef_search: usize,
	context_window: usize
) -> Result<Vec<SemanticChunk>, String> {
	with_search_tree(|search_tree| {
		search_tree
			.search_hierarchical(query_vector, top_parent_k, ef_search, context_window)
			.map(|found| found.into_iter().cloned().collect())
	})
}

pub fn query_search_tree_with_previous_context(
	query_vector: &[f32],
	top_parent_k: usize,
	ef_search: usize,
	context_window: usize,
	relation_threshold: f32
) -> Result<ContextSearchResult, String> {
	with_search_tree(|search_tree| {
		search_tree
			.search_with_previous_context(
				query_vector,
				top_parent_k,
				ef_search,
				context_window,
				relation_threshold
			)
			.map(|found| ContextSearchResult {
				chunks: found.chunks.into_iter().cloned().collect(),
				related_level: found.related_level,
			})
	})
}

/// Chunks of `level` (2 = paragraphs) that contain the previous search's matches.
pub fn previous_context_chunks(level: usize) -> Vec<SemanticChunk> {
	with_search_tree(|search_tree| {
		search_tree.previous_context_chunks(level).into_iter().cloned().collect()
	})
}

pub fn reset_previous_context() {
	with_search_tree(|search_tree| search_tree.reset_previous_context());
}

pub fn search_presentation(
	query_vector: &[f32],
	top_parent_k: usize,
	ef_search: usize,
	context_window: usize,
	relation_threshold: f32
) -> Result<ContextHit, String> {
	with_search_tree(
		|tree| -> Result<ContextHit, String> {
			let had_previous = tree.has_previous_context();
			let previous_chunks: Vec<String> = tree
				.previous_context_chunks(1)
				.into_iter()
				.map(|c| c.text.clone())
				.collect();

			let found = tree.search_with_previous_context(
				query_vector,
				top_parent_k,
				ef_search,
				context_window,
				relation_threshold
			)?;

			Ok(ContextHit {
				related_level: found.related_level,
				topic_shift: had_previous && found.related_level.is_none(),
				chunks: found.chunks.into_iter().cloned().collect(),
				previous_chunks,
			})
		}
	)
}
