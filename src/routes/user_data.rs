use crate::{db::{Database, UserProfile}, AppState};
use axum::{extract::{Path, State}, http::StatusCode, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn profiles_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_profiles).post(save_profile))
        .route("/active", axum::routing::put(set_active_profile))
        .route("/:id", axum::routing::delete(delete_profile))
}

pub fn preferences_router() -> Router<Arc<AppState>> {
    Router::new().route("/theme", axum::routing::put(set_theme))
}

#[derive(Serialize)]
struct ProfilesResponse {
    profiles: Vec<UserProfile>,
    active_profile_id: Option<String>,
    theme: String,
}

async fn list_profiles(State(state): State<Arc<AppState>>) -> Result<Json<ProfilesResponse>, ApiError> {
    let db = open_db(&state)?;
    Ok(Json(ProfilesResponse {
        profiles: db.get_profiles().map_err(ApiError::internal)?,
        active_profile_id: db.get_preference("active_profile_id").map_err(ApiError::internal)?.filter(|id| !id.is_empty()),
        theme: db.get_preference("theme").map_err(ApiError::internal)?.unwrap_or_else(|| state.config.ui.theme.clone()),
    }))
}

async fn save_profile(
    State(state): State<Arc<AppState>>,
    Json(profile): Json<UserProfile>,
) -> Result<Json<UserProfile>, ApiError> {
    if profile.id.trim().is_empty() || profile.name.trim().is_empty() {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Profile id and name are required".into()));
    }
    open_db(&state)?.save_profile(&profile).map_err(ApiError::internal)?;
    Ok(Json(profile))
}

#[derive(Deserialize)]
struct ActiveProfilePayload { active_profile_id: Option<String> }

async fn set_active_profile(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ActiveProfilePayload>,
) -> Result<StatusCode, ApiError> {
    let db = open_db(&state)?;
    if let Some(id) = payload.active_profile_id {
        if !db.get_profiles().map_err(ApiError::internal)?.iter().any(|profile| profile.id == id) {
            return Err(ApiError(StatusCode::NOT_FOUND, "Profile not found".into()));
        }
        db.set_preference("active_profile_id", &id).map_err(ApiError::internal)?;
    } else {
        db.set_preference("active_profile_id", "").map_err(ApiError::internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_profile(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    open_db(&state)?.delete_profile(&id).map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ThemePayload { theme: String }

async fn set_theme(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ThemePayload>,
) -> Result<StatusCode, ApiError> {
    if payload.theme != "dark" && payload.theme != "light" {
        return Err(ApiError(StatusCode::BAD_REQUEST, "Theme must be 'dark' or 'light'".into()));
    }
    open_db(&state)?.set_preference("theme", &payload.theme).map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

fn open_db(state: &AppState) -> Result<Database, ApiError> {
    Database::init(state.data_dir.join("sessions/chats.db")).map_err(ApiError::internal)
}

struct ApiError(StatusCode, String);
impl ApiError {
    fn internal(error: impl std::fmt::Display) -> Self { Self(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()) }
}
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response { (self.0, self.1).into_response() }
}
