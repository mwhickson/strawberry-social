use crate::AppState;
use axum::Router;
use std::sync::Arc;

pub mod application;
pub mod characters;
pub mod chat;
pub mod config_route;
pub mod models;
pub mod runtime_info;
pub mod runtime_server;
pub mod user_data;

pub fn create_router(_state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .nest("/api/characters", super::routes::characters::router())
        .nest("/api/chat", super::routes::chat::router())
        .nest("/api/config", super::routes::config_route::router()) // <-- Wire it up here!
        .nest("/api/application", super::routes::application::router())
        .nest("/api/profiles", super::routes::user_data::profiles_router())
        .nest(
            "/api/preferences",
            super::routes::user_data::preferences_router(),
        )
        .nest("/api/runtime-info", super::routes::runtime_info::router())
        .nest(
            "/api/runtime-server",
            super::routes::runtime_server::router(),
        )
        .nest("/api/models", super::routes::models::router())
}
