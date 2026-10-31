use axum::{
    routing::get,
    Json, Router,
};
use std::fs;
use std::path::Path;
use crate::card::CharacterCard;
use base64::{engine::general_purpose, Engine as _};

pub fn router() -> Router<std::sync::Arc<crate::AppState>> {
    Router::new()
        .route("/", get(list_characters))
}

#[derive(serde::Serialize)]
pub struct CharacterSummary {
    filename: String,
    name: String,
    description: String,
    creator: String,
    avatar_url: Option<String>,
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
                                // Generate base64 data URL for the avatar image
                                let avatar_url = if ext == "png" {
                                    match fs::read(&path) {
                                        Ok(bytes) => Some(format!(
                                            "data:image/png;base64,{}",
                                            general_purpose::STANDARD.encode(&bytes)
                                        )),
                                        Err(_) => None,
                                    }
                                } else {
                                    let png_path = path.with_extension("png");
                                    if png_path.exists() {
                                        match fs::read(&png_path) {
                                            Ok(bytes) => Some(format!(
                                                "data:image/png;base64,{}",
                                                general_purpose::STANDARD.encode(&bytes)
                                            )),
                                            Err(_) => None,
                                        }
                                    } else {
                                        None
                                    }
                                };

                                summaries.push(CharacterSummary {
                                    filename: path.file_name().unwrap().to_string_lossy().to_string(),
                                    name: card.data.name,
                                    description: card.data.description,
                                    creator: card.data.creator,
                                    avatar_url,
                                });
                            }
                            Err(e) => {
                                tracing::error!("Failed to parse card {}: {}", path.display(), e);
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
