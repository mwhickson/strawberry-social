use crate::card::CharacterCard;
use axum::{
    extract::{Multipart, State},
    routing::get,
    Json, Router,
};
use base64::{engine::general_purpose, Engine as _};
use std::fs;
use std::path::Path;
use std::sync::Arc;

pub fn router() -> Router<Arc<crate::AppState>> {
    Router::new()
        .route("/", get(list_characters).post(upload_character))
        .route(
            "/:hash/speech-settings",
            get(get_speech_settings)
                .put(save_speech_settings)
                .delete(reset_speech_settings),
        )
}

#[derive(serde::Serialize)]
pub struct CharacterSummary {
    id: String,
    filename: String,
    name: String,
    description: String,
    creator: String,
    avatar_url: Option<String>,
}

async fn list_characters(State(state): State<Arc<crate::AppState>>) -> Json<Vec<CharacterSummary>> {
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
                                let id = fs::read(&path)
                                    .map(|bytes| crate::card::content_hash(&bytes))
                                    .unwrap_or_default();
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
                                    id,
                                    filename: path
                                        .file_name()
                                        .unwrap()
                                        .to_string_lossy()
                                        .to_string(),
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
            StatusCodeResponse(
                axum::http::StatusCode::BAD_REQUEST,
                format!("Invalid character card: {}", e),
            )
        })?;
        let character_hash = crate::card::content_hash(&data);

        let avatar_url = if filename.to_lowercase().ends_with(".png") {
            Some(format!(
                "data:image/png;base64,{}",
                general_purpose::STANDARD.encode(&data)
            ))
        } else {
            None
        };

        return Ok(Json(CharacterSummary {
            id: character_hash,
            filename,
            name: card.data.name,
            description: card.data.description,
            creator: card.data.creator,
            avatar_url,
        }));
    }

    Err(StatusCodeResponse(
        axum::http::StatusCode::BAD_REQUEST,
        "No file uploaded".into(),
    ))
}

#[derive(serde::Deserialize)]
struct SpeechSettingsPayload {
    voice_uri: String,
    rate: f64,
    pitch: f64,
}

async fn get_speech_settings(
    State(state): State<Arc<crate::AppState>>,
    axum::extract::Path(hash): axum::extract::Path<String>,
) -> Result<Json<Option<crate::db::CharacterSpeechSettings>>, StatusCodeResponse> {
    validate_character_hash(&hash)?;
    let db = crate::db::Database::init(state.data_dir.join("sessions/chats.db"))
        .map_err(internal_error)?;
    Ok(Json(
        db.get_character_speech_settings(&hash)
            .map_err(internal_error)?,
    ))
}

async fn save_speech_settings(
    State(state): State<Arc<crate::AppState>>,
    axum::extract::Path(hash): axum::extract::Path<String>,
    Json(payload): Json<SpeechSettingsPayload>,
) -> Result<Json<crate::db::CharacterSpeechSettings>, StatusCodeResponse> {
    validate_character_hash(&hash)?;
    if payload.voice_uri.len() > 2048
        || !payload.rate.is_finite()
        || !(0.1..=2.0).contains(&payload.rate)
        || !payload.pitch.is_finite()
        || !(0.0..=2.0).contains(&payload.pitch)
    {
        return Err(StatusCodeResponse(
            axum::http::StatusCode::BAD_REQUEST,
            "Choose a voice URI up to 2048 characters, a rate from 0.1 to 2, and a pitch from 0 to 2.".into(),
        ));
    }
    let settings = crate::db::CharacterSpeechSettings {
        voice_uri: payload.voice_uri,
        rate: payload.rate,
        pitch: payload.pitch,
    };
    let db = crate::db::Database::init(state.data_dir.join("sessions/chats.db"))
        .map_err(internal_error)?;
    db.save_character_speech_settings(&hash, &settings)
        .map_err(internal_error)?;
    Ok(Json(settings))
}

async fn reset_speech_settings(
    State(state): State<Arc<crate::AppState>>,
    axum::extract::Path(hash): axum::extract::Path<String>,
) -> Result<axum::http::StatusCode, StatusCodeResponse> {
    validate_character_hash(&hash)?;
    let db = crate::db::Database::init(state.data_dir.join("sessions/chats.db"))
        .map_err(internal_error)?;
    db.delete_character_speech_settings(&hash)
        .map_err(internal_error)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn validate_character_hash(hash: &str) -> Result<(), StatusCodeResponse> {
    if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(StatusCodeResponse(
            axum::http::StatusCode::BAD_REQUEST,
            "Invalid character card hash".into(),
        ));
    }
    Ok(())
}

struct StatusCodeResponse(axum::http::StatusCode, String);
impl axum::response::IntoResponse for StatusCodeResponse {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}

fn internal_error<E: std::fmt::Display>(err: E) -> StatusCodeResponse {
    StatusCodeResponse(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        err.to_string(),
    )
}
