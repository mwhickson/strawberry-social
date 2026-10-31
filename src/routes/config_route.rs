use axum::{routing::get, Json, Router};
use std::sync::Arc;
use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/", get(get_config))
}

async fn get_config(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<crate::config::UiConfig> {
    Json(state.config.ui.clone())
}