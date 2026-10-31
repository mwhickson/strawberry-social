use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use rust_embed::RustEmbed;
use std::fs::File;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use directories::ProjectDirs;

mod config;
mod card;
mod db;
mod llm;
mod routes;

use config::AppConfig;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Asset;

pub struct AppState {
    pub config: AppConfig,
    pub data_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Determine platform-standard data directory
    // Linux: ~/.local/share/strawberry-social
    // Windows: C:\Users\<User>\AppData\Local\StrawberrySocial
    let proj_dirs = ProjectDirs::from("com", "StrawberrySocial", "StrawberrySocial")
        .ok_or("Failed to determine home directory")?;

    let data_dir = proj_dirs.data_local_dir();
    std::fs::create_dir_all(data_dir)?;

    // Define absolute paths for subdirectories and files
    let characters_dir = data_dir.join("characters");
    let avatars_dir = characters_dir.join("avatars");
    let lorebooks_dir = characters_dir.join("lorebooks");
    let sessions_dir = data_dir.join("sessions");
    let config_path = data_dir.join("config.yaml");
    let log_path = data_dir.join("strawberry.log");

    // Ensure platform directories exist
    for dir in &[&characters_dir, &avatars_dir, &lorebooks_dir, &sessions_dir] {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
    }

    // Initialize logging to file in the data directory
    let log_file = File::create(&log_path)?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(log_file).with_ansi(false))
        .init();

    // Load or generate config.yaml in the standard directory
    let config = AppConfig::load_or_create(&config_path)?;
    let state = Arc::new(AppState {
        config: config.clone(),
        data_dir: data_dir.to_path_buf(),
    });

    // Build Axum router combining API routes, config, and static asset fallback
    let app = Router::new()
        .route("/api/health", get(|| async { "OK" }))
        .merge(routes::create_router(state.clone()))
        .fallback(static_handler)
        .with_state(state.clone());

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    tracing::info!("Strawberry Social starting on http://{}", addr);

    if config.server.open_browser_on_start {
        let _ = open::that(format!("http://{}", addr));
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// Handler for embedded static files (index.html, CSS, JS)
async fn static_handler(uri: Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        path = "index.html";
    }

    match Asset::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap())
                .body(Body::from(content.data))
                .unwrap()
        }
        None => {
            if let Some(content) = Asset::get("index.html") {
                Response::builder()
                    .header(header::CONTENT_TYPE, HeaderValue::from_static("text/html"))
                    .body(Body::from(content.data))
                    .unwrap()
            } else {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("404 Not Found"))
                    .unwrap()
            }
        }
    }
}