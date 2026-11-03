use docling::{ DocumentConverter, SourceDocument };
use tauri::Runtime;
use uuid::Uuid;
use crate::llm::chunker::utils::{
	append_search_tree_layer,
	chunk_text,
	combine_chunks,
	store_chunks,
};

#[tauri::command]
pub async fn create_and_send_chunks(file_paths: Vec<String>) -> Result<(), String> {
	let converter: DocumentConverter = DocumentConverter::new();

	for path in file_paths {
		let source = SourceDocument::from_file(path).map_err(|e| e.to_string())?;
		let processed = converter.convert(source).map_err(|e| e.to_string())?;
		let markdown = processed.document.export_to_markdown();

		let document_id = Uuid::new_v4().to_string();

		// Each combine_chunks call also sets parent_id on the chunks it consumes.
		let mut sentences = chunk_text(&markdown).map_err(|e| e.to_string())?;
		let mut sentence_chunks = combine_chunks(&mut sentences, 0.9).map_err(|e| e.to_string())?;
		let mut paragraph_chunks = combine_chunks(&mut sentence_chunks, 0.72).map_err(|e|
			e.to_string()
		)?;
		let mut section_chunks = combine_chunks(&mut paragraph_chunks, 0.6).map_err(|e|
			e.to_string()
		)?;

		// Sections point at the document.
		for section in section_chunks.iter_mut() {
			section.parent_id = Some(document_id.clone());
		}

		// Populate only once every parent_id is final.
		let layers = [&sentences, &sentence_chunks, &paragraph_chunks, &section_chunks];
		for (layer_idx, chunks) in layers.into_iter().enumerate() {
			append_search_tree_layer(layer_idx, chunks)?;
			store_chunks(layer_idx, chunks);
		}
	}

	Ok(())
}

#[tauri::command]
pub async fn load_data_for_notebook<R: Runtime>(
	_app: tauri::AppHandle<R>,
	_window: tauri::Window<R>
) -> Result<(), String> {
	Ok(())
}
