use axum::Router;
use std::sync::Arc;
use crate::AppState;

pub mod characters;
pub mod chat;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .nest("/api/characters", characters::router())
        .nest("/api/chat", chat::router())
        .with_state(state)
}