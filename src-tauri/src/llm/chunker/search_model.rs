use std::cmp::Ordering;
use std::collections::{ BTreeSet, HashMap, HashSet };

use usearch::{ Index, IndexOptions, MetricKind, ScalarKind };

use crate::llm::chunker::types::SemanticChunk;

// --------------------------------------------------
// Configuration
// --------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct LayerConfig {
	pub max_elements: usize,
	pub m: usize,
	pub ef_construction: usize,
}

impl LayerConfig {
	pub fn new(max_elements: usize, m: usize, ef_construction: usize) -> Self {
		Self {
			max_elements,
			m,
			ef_construction,
		}
	}
}

/// Result of `search_with_previous_context`.
pub struct ContextSearch<'a> {
	pub chunks: Vec<&'a SemanticChunk>,
	/// Layer at which the query was related to the previous context
	/// (0 = sentences ... N-1 = broadest). None = unrelated at every level,
	/// so a global search was used.
	pub related_level: Option<usize>,
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
	let mut dot = 0.0f32;
	let mut na = 0.0f32;
	let mut nb = 0.0f32;
	for (x, y) in a.iter().zip(b.iter()) {
		dot += x * y;
		na += x * x;
		nb += y * y;
	}
	if na == 0.0 || nb == 0.0 {
		0.0
	} else {
		dot / (na.sqrt() * nb.sqrt())
	}
}

// --------------------------------------------------
// Layer
// --------------------------------------------------

pub struct SearchLayer {
	pub index: Index,
	pub chunks: Vec<SemanticChunk>,

	// USearch key -> position in chunks
	key_to_position: HashMap<u64, usize>,
	// chunk.id -> position in chunks
	id_to_position: HashMap<String, usize>,
	next_key: u64,

	config: LayerConfig,
}

impl SearchLayer {
	fn new(dim: usize, config: LayerConfig) -> Result<Self, String> {
		let options = IndexOptions {
			dimensions: dim,
			metric: MetricKind::Cos,
			quantization: ScalarKind::F32,
			connectivity: config.m,
			expansion_add: config.ef_construction,
			expansion_search: 0,
			..Default::default()
		};

		let index = Index::new(&options).map_err(|e| e.to_string())?;

		index.reserve(config.max_elements).map_err(|e| e.to_string())?;

		Ok(Self {
			index,
			chunks: Vec::new(),
			key_to_position: HashMap::new(),
			id_to_position: HashMap::new(),
			next_key: 1,
			config,
		})
	}

	/// Add chunks after the existing ones (positions of old chunks stay valid).
	fn append(&mut self, chunks: Vec<SemanticChunk>) -> Result<(), String> {
		let needed = self.chunks.len() + chunks.len();
		let capacity = self.index.capacity();

		// USearch has a fixed capacity; grow it when a layer overflows.
		if needed > capacity {
			let target = needed.max(capacity.saturating_mul(2));
			self.index.reserve(target).map_err(|e| e.to_string())?;
		}

		for chunk in chunks {
			// Internal keys are unique and independent of chunk.id.
			let key = self.next_key;

			self.index.add(key, &chunk.embedding).map_err(|e| e.to_string())?;

			let position = self.chunks.len();
			self.key_to_position.insert(key, position);
			self.id_to_position.insert(chunk.id.clone(), position);
			self.chunks.push(chunk);
			self.next_key += 1;
		}

		Ok(())
	}

	/// Replace all contents of this layer.
	fn populate(&mut self, chunks: Vec<SemanticChunk>) -> Result<(), String> {
		self.reset()?;
		self.append(chunks)
	}

	/// Reset this layer while retaining its configuration.
	fn reset(&mut self) -> Result<(), String> {
		self.index.reset().map_err(|e| e.to_string())?;

		self.index.reserve(self.config.max_elements).map_err(|e| e.to_string())?;

		self.chunks.clear();
		self.key_to_position.clear();
		self.id_to_position.clear();
		self.next_key = 1;

		Ok(())
	}
}

// --------------------------------------------------
// Hierarchical Search Tree
// --------------------------------------------------

pub struct HierarchicalSearchTree {
	pub layers: Vec<SearchLayer>,
	pub dimensions: usize,
	/// Fine-layer (layer 0) positions matched by the previous
	/// `search_with_previous_context` call.
	previous_context: Vec<usize>,
}

impl HierarchicalSearchTree {
	/// Configurations are ordered:
	/// Layer 0 = finest
	/// Layer N-1 = broadest
	pub fn new(dim: usize, configs: &[LayerConfig]) -> Result<Self, String> {
		let mut layers = Vec::with_capacity(configs.len());

		for config in configs {
			layers.push(SearchLayer::new(dim, *config)?);
		}

		Ok(Self {
			layers,
			dimensions: dim,
			previous_context: Vec::new(),
		})
	}

	fn validate(&self, chunks: &[SemanticChunk]) -> Result<(), String> {
		for chunk in chunks {
			if chunk.embedding.len() != self.dimensions {
				return Err(
					format!(
						"Invalid embedding dimension: expected {}, got {}",
						self.dimensions,
						chunk.embedding.len()
					)
				);
			}
		}
		Ok(())
	}

	// --------------------------------------------------
	// Populate
	// --------------------------------------------------

	/// Replace the contents of a layer.
	pub fn populate_layer(
		&mut self,
		layer_idx: usize,
		chunks: Vec<SemanticChunk>
	) -> Result<(), String> {
		self.validate(&chunks)?;

		let layer = self.layers.get_mut(layer_idx).ok_or("Invalid layer index")?;
		layer.populate(chunks)?;

		self.previous_context.clear();
		Ok(())
	}

	/// Add chunks to a layer without removing what is already there
	/// (use this to index several documents).
	pub fn append_layer(
		&mut self,
		layer_idx: usize,
		chunks: Vec<SemanticChunk>
	) -> Result<(), String> {
		self.validate(&chunks)?;

		let layer = self.layers.get_mut(layer_idx).ok_or("Invalid layer index")?;
		layer.append(chunks)
	}

	// --------------------------------------------------
	// Reset
	// --------------------------------------------------

	/// Reset a specific layer.
	pub fn reset_layer(&mut self, layer_idx: usize) -> Result<(), String> {
		let layer = self.layers.get_mut(layer_idx).ok_or("Invalid layer index")?;
		layer.reset()?;

		self.previous_context.clear();
		Ok(())
	}

	/// Reset every layer.
	pub fn reset_all_layers(&mut self) -> Result<(), String> {
		for layer in &mut self.layers {
			layer.reset()?;
		}

		self.previous_context.clear();
		Ok(())
	}

	/// Forget the previous search context (e.g. when a new conversation starts).
	pub fn reset_previous_context(&mut self) {
		self.previous_context.clear();
	}

	// --------------------------------------------------
	// Tree helpers
	// --------------------------------------------------

	/// Position of the ancestor (in layer `target`) of a fine chunk (layer 0).
	fn ancestor_at(&self, fine_pos: usize, target: usize) -> Option<usize> {
		let mut pos = fine_pos;
		for level in 0..target {
			let parent_id = self.layers[level].chunks[pos].parent_id.as_deref()?;
			pos = *self.layers[level + 1].id_to_position.get(parent_id)?;
		}
		Some(pos)
	}

	/// Document id of a fine chunk (parent of its section chunk).
	fn document_id(&self, fine_pos: usize) -> Option<&str> {
		let top = self.layers.len().checked_sub(1)?;
		let top_pos = self.ancestor_at(fine_pos, top)?;
		self.layers[top].chunks[top_pos].parent_id.as_deref()
	}

	/// All layer-0 positions that descend from the given chunks of `from_layer`.
	fn descend(&self, from_layer: usize, positions: &HashSet<usize>) -> HashSet<usize> {
		let mut current = positions.clone();

		for level in (0..from_layer).rev() {
			let parent_ids: HashSet<&str> = current
				.iter()
				.map(|&p| self.layers[level + 1].chunks[p].id.as_str())
				.collect();

			current = self.layers[level].chunks
				.iter()
				.enumerate()
				.filter(|(_, c)| c.parent_id.as_deref().map_or(false, |p| parent_ids.contains(p)))
				.map(|(i, _)| i)
				.collect();
		}

		current
	}

	fn check_query(&self, query_vector: &[f32]) -> Result<(), String> {
		if query_vector.len() != self.dimensions {
			return Err("Query embedding dimension mismatch".into());
		}
		Ok(())
	}

	/// Search the broadest layer and resolve hits to fine-chunk positions.
	fn global_matches(
		&self,
		query_vector: &[f32],
		top_parent_k: usize,
		ef_search: usize
	) -> Result<Vec<usize>, String> {
		let Some(broadest_idx) = self.layers.len().checked_sub(1) else {
			return Ok(Vec::new());
		};
		let broadest = &self.layers[broadest_idx];

		if top_parent_k == 0 || broadest.chunks.is_empty() || self.layers[0].chunks.is_empty() {
			return Ok(Vec::new());
		}

		broadest.index.change_expansion_search(ef_search.max(top_parent_k));

		let hits = broadest.index.search(query_vector, top_parent_k).map_err(|e| e.to_string())?;

		let parents: HashSet<usize> = hits.keys
			.iter()
			.filter_map(|key| broadest.key_to_position.get(key).copied())
			.collect();

		let mut matched: Vec<usize> = self.descend(broadest_idx, &parents).into_iter().collect();
		matched.sort_unstable();
		Ok(matched)
	}

	/// Check the previous context from the finest level upward. At each level
	/// the query is compared with the previous context's chunks of that level;
	/// if none is related enough we move up to their parents. Once a level is
	/// good enough, the search runs only inside the chunks that matched at
	/// that level. Returns None when no level (up to the broadest) is related.
	fn scoped_matches(
		&self,
		query_vector: &[f32],
		relation_threshold: f32,
		top_k: usize
	) -> Option<(usize, Vec<usize>)> {
		if self.previous_context.is_empty() || top_k == 0 {
			return None;
		}

		for level in 0..self.layers.len() {
			let ancestors: HashSet<usize> = self.previous_context
				.iter()
				.filter_map(|&p| self.ancestor_at(p, level))
				.collect();

			let qualifying: HashSet<usize> = ancestors
				.into_iter()
				.filter(|&p| {
					cosine(query_vector, &self.layers[level].chunks[p].embedding) >=
						relation_threshold
				})
				.collect();

			if qualifying.is_empty() {
				continue;
			}

			// Related enough at this level: search inside it.
			let scope = self.descend(level, &qualifying);
			let mut scored: Vec<(usize, f32)> = scope
				.into_iter()
				.map(|p| (p, cosine(query_vector, &self.layers[0].chunks[p].embedding)))
				.collect();

			scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
			scored.truncate(top_k);

			let mut matched: Vec<usize> = scored
				.into_iter()
				.map(|(p, _)| p)
				.collect();
			matched.sort_unstable();
			return Some((level, matched));
		}

		None
	}

	/// Add neighbouring fine chunks (same document only) and return the
	/// chunks in document order.
	fn expand_with_context(&self, matched: &[usize], context_window: usize) -> Vec<&SemanticChunk> {
		if matched.is_empty() {
			return Vec::new();
		}

		let fine = &self.layers[0].chunks;
		let last = fine.len() - 1;
		let mut positions = BTreeSet::new();

		for &position in matched {
			positions.insert(position);

			if context_window == 0 {
				continue;
			}

			let doc = self.document_id(position);
			let start = position.saturating_sub(context_window);
			let end = position.saturating_add(context_window).min(last);

			for neighbor in start..=end {
				if neighbor != position && self.document_id(neighbor) == doc {
					positions.insert(neighbor);
				}
			}
		}

		positions
			.into_iter()
			.map(|p| &fine[p])
			.collect()
	}

	// --------------------------------------------------
	// Hierarchical Search
	// --------------------------------------------------

	/// Search the broadest layer, resolve the hits down to fine chunks via
	/// parent ids, then include neighbouring fine chunks.
	///
	/// context_window = 0: Only matching chunks.
	/// context_window = 1: One chunk before and after.
	/// context_window = 2: Two chunks before and after.
	pub fn search_hierarchical(
		&self,
		query_vector: &[f32],
		top_parent_k: usize,
		ef_search: usize,
		context_window: usize
	) -> Result<Vec<&SemanticChunk>, String> {
		self.check_query(query_vector)?;

		let matched = self.global_matches(query_vector, top_parent_k, ef_search)?;

		Ok(self.expand_with_context(&matched, context_window))
	}

	// --------------------------------------------------
	// Search with previous context
	// --------------------------------------------------

	/// Like `search_hierarchical`, but first checks how the query relates to
	/// the previous search (see `scoped_matches`). `relation_threshold` is the
	/// minimum cosine similarity for a level to count as related. If nothing
	/// is related (or there is no previous context) it falls back to a global
	/// search. The result becomes the new previous context.
	pub fn search_with_previous_context(
		&mut self,
		query_vector: &[f32],
		top_parent_k: usize,
		ef_search: usize,
		context_window: usize,
		relation_threshold: f32
	) -> Result<ContextSearch<'_>, String> {
		self.check_query(query_vector)?;

		let (related_level, matched) = match
			self.scoped_matches(query_vector, relation_threshold, top_parent_k)
		{
			Some((level, m)) => (Some(level), m),
			None => (None, self.global_matches(query_vector, top_parent_k, ef_search)?),
		};

		if !matched.is_empty() {
			self.previous_context = matched.clone();
		}

		Ok(ContextSearch {
			chunks: self.expand_with_context(&matched, context_window),
			related_level,
		})
	}

	/// Chunks of layer `level` that contain the previous search's matches
	/// (document order). Read this before running the next search.
	pub fn previous_context_chunks(&self, level: usize) -> Vec<&SemanticChunk> {
		if level >= self.layers.len() {
			return Vec::new();
		}

		let positions: BTreeSet<usize> = self.previous_context
			.iter()
			.filter_map(|&p| self.ancestor_at(p, level))
			.collect();

		positions
			.into_iter()
			.map(|p| &self.layers[level].chunks[p])
			.collect()
	}

	pub fn has_previous_context(&self) -> bool {
		!self.previous_context.is_empty()
	}
}
