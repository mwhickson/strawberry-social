use crate::AppState;
use axum::{routing::get, Json, Router};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/", get(get_config))
}

async fn get_config(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "api_base_url": state.config.llm.api_base_url,
        "theme": state.config.ui.theme,
    }))
}
