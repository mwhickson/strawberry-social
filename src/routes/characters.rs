use axum::{
    extract::{Multipart, State},
    routing::get,
    Json, Router,
};
use std::fs;
use std::path::Path;
use crate::card::CharacterCard;
use base64::{engine::general_purpose, Engine as _};
use std::sync::Arc;

pub fn router() -> Router<Arc<crate::AppState>> {
    Router::new()
        .route("/", get(list_characters).post(upload_character))
}

#[derive(serde::Serialize)]
pub struct CharacterSummary {
    filename: String,
    name: String,
    description: String,
    creator: String,
    avatar_url: Option<String>,
}

async fn list_characters(
    State(state): State<Arc<crate::AppState>>,
) -> Json<Vec<CharacterSummary>> {
    let mut summaries = Vec::new();
    let root_dir = state.data_dir.join("characters");

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

    scan_directory(&root_dir, &mut summaries);
    Json(summaries)
}

async fn upload_character(
    State(state): State<Arc<crate::AppState>>,
    mut multipart: Multipart,
) -> Result<Json<CharacterSummary>, StatusCodeResponse> {
    let characters_dir = state.data_dir.join("characters");
    fs::create_dir_all(&characters_dir).map_err(internal_error)?;

    while let Some(field) = multipart.next_field().await.map_err(internal_error)? {
        let filename = field.file_name().unwrap_or("card.json").to_string();
        let data = field.bytes().await.map_err(internal_error)?;

        let dest_path = characters_dir.join(&filename);
        fs::write(&dest_path, &data).map_err(internal_error)?;

        // Validate card
        let card = CharacterCard::from_file(&dest_path).map_err(|e| {
            let _ = fs::remove_file(&dest_path);
            StatusCodeResponse(axum::http::StatusCode::BAD_REQUEST, format!("Invalid character card: {}", e))
        })?;

        let avatar_url = if filename.to_lowercase().ends_with(".png") {
            Some(format!(
                "data:image/png;base64,{}",
                general_purpose::STANDARD.encode(&data)
            ))
        } else {
            None
        };

        return Ok(Json(CharacterSummary {
            filename,
            name: card.data.name,
            description: card.data.description,
            creator: card.data.creator,
            avatar_url,
        }));
    }

    Err(StatusCodeResponse(axum::http::StatusCode::BAD_REQUEST, "No file uploaded".into()))
}

struct StatusCodeResponse(axum::http::StatusCode, String);
impl axum::response::IntoResponse for StatusCodeResponse {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}

fn internal_error<E: std::fmt::Display>(err: E) -> StatusCodeResponse {
    StatusCodeResponse(axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}