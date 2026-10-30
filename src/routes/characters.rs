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
    let dir = Path::new("characters");

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(card) = CharacterCard::from_file(&path) {
                    summaries.push(CharacterSummary {
                        filename: path.file_name().unwrap().to_string_lossy().to_string(),
                        name: card.data.name,
                        description: card.data.description,
                        creator: card.data.creator,
                    });
                }
            }
        }
    }

    Json(summaries)
}