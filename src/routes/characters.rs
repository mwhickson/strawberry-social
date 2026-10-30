use axum::{
    routing::get,
    Json, Router,
};
use std::fs;
use std::path::Path;
use crate::card::CharacterCard;

pub fn router() -> Router<std::sync::Arc<crate::AppState>> {
    Router::new().route("/", get(list_characters))
}

#[derive(serde::Serialize)]
pub struct CharacterSummary {
    filename: String,
    name: String,
    description: String,
    creator: String,
}

async fn list_characters() -> Json<Vec<CharacterSummary>> {
    let mut summaries = Vec::new();
    let root_dir = Path::new("characters");

    // Helper to recursively scan directories for cards
    fn scan_directory(dir: &Path, summaries: &mut Vec<CharacterSummary>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    scan_directory(&path, summaries);
                } else if path.is_file() {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();

                    if ext == "json" || ext == "png" {
                        match CharacterCard::from_file(&path) {
                            Ok(card) => {
                                summaries.push(CharacterSummary {
                                    filename: path.file_name().unwrap().to_string_lossy().to_string(),
                                    name: card.data.name,
                                    description: card.data.description,
                                    creator: card.data.creator,
                                });
                            }
                            Err(e) => {
                                eprintln!("Failed to parse card {}: {}", path.display(), e);
                            }
                        }
                    }
                }
            }
        }
    }

    scan_directory(root_dir, &mut summaries);
    Json(summaries)
}