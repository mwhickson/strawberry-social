use axum::Router;
use std::sync::Arc;
use crate::AppState;

pub mod characters;
pub mod chat;
pub mod config_route;
pub mod user_data;

pub fn create_router(_state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .nest("/api/characters", super::routes::characters::router())
        .nest("/api/chat", super::routes::chat::router())
        .nest("/api/config", super::routes::config_route::router()) // <-- Wire it up here!
        .nest("/api/profiles", super::routes::user_data::profiles_router())
        .nest("/api/preferences", super::routes::user_data::preferences_router())
}
