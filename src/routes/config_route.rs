use crate::AppState;
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Deserialize;
use std::{fs, sync::Arc};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(get_config))
        .route("/generation", axum::routing::put(update_generation))
}

async fn get_config(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let generation = *state.generation.read().await;
    Json(serde_json::json!({
        "api_base_url": state.config.llm.api_base_url,
        "theme": state.config.ui.theme,
        "context_window": generation.context_window,
        "temperature": generation.temperature,
        "top_p": generation.top_p,
    }))
}

#[derive(Deserialize)]
struct GenerationPayload {
    context_window: usize,
    temperature: f32,
    top_p: f32,
}

async fn update_generation(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<GenerationPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !(256..=262_144).contains(&payload.context_window)
        || payload.context_window % 256 != 0
        || !payload.temperature.is_finite()
        || !(0.0..=2.0).contains(&payload.temperature)
        || !payload.top_p.is_finite()
        || !(0.01..=1.0).contains(&payload.top_p)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Choose a context window from 256 to 262144 in steps of 256, a temperature from 0 to 2, and a top-p value from 0.01 to 1.".into(),
        ));
    }

    let mut generation = state.generation.write().await;
    let config_path = state.data_dir.join("config.yaml");
    let mut config = crate::config::AppConfig::load_or_create(&config_path)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    config.llm.context_window = payload.context_window;
    config.llm.temperature = payload.temperature;
    config.llm.top_p = payload.top_p;
    let yaml = serde_yaml::to_string(&config)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    fs::write(&config_path, yaml)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    *generation = crate::config::GenerationSettings {
        context_window: payload.context_window,
        temperature: payload.temperature,
        top_p: payload.top_p,
    };
    Ok(Json(serde_json::json!({
        "context_window": generation.context_window,
        "temperature": generation.temperature,
        "top_p": generation.top_p,
    })))
}
