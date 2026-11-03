use fastembed::{ EmbeddingModel, TextEmbedding, TextInitOptions };
use std::sync::Mutex;
use crate::llm::chunker::search_model::{ HierarchicalSearchTree, LayerConfig };

static EMBEDDING_MODEL: Mutex<Option<TextEmbedding>> = Mutex::new(None);
static SEARCH_TREE: Mutex<Option<HierarchicalSearchTree>> = Mutex::new(None);

pub fn with_embedding_model<F, R>(f: F) -> R where F: FnOnce(&mut TextEmbedding) -> R {
	let mut guard = EMBEDDING_MODEL.lock().unwrap();

	let model = guard.get_or_insert_with(|| {
		TextEmbedding::try_new(
			TextInitOptions::new(EmbeddingModel::BGEM3).with_show_download_progress(
				cfg!(debug_assertions)
			)
		).expect("Failed to load embedding model")
	});

	f(model)
}

pub fn with_search_tree<F, R>(f: F) -> R where F: FnOnce(&mut HierarchicalSearchTree) -> R {
	let mut guard = SEARCH_TREE.lock().unwrap();

	let search_tree = guard.get_or_insert_with(|| {
		let dim = TextEmbedding::get_model_info(&EmbeddingModel::BGEM3)
			.expect("Failed to read embedding model info").dim;

		// Layer 0 = sentences, 1 = sentence groups, 2 = paragraphs, 3 = sections.
		HierarchicalSearchTree::new(
			dim,
			&[
				LayerConfig::new(5000, 100, 1000),
				LayerConfig::new(5000, 100, 1000),
				LayerConfig::new(5000, 100, 1000),
				LayerConfig::new(5000, 100, 1000),
			]
		).expect("Failed to create search tree")
	});

	f(search_tree)
}
