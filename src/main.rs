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
use std::path::Path;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging to file: strawberry.log
    let log_file = File::create("strawberry.log")?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(log_file).with_ansi(false))
        .init();

    // Ensure local directories exist
    let dirs = ["characters", "characters/avatars", "characters/lorebooks", "sessions"];
    for dir in dirs.iter() {
        if !Path::new(dir).exists() {
            std::fs::create_dir_all(dir)?;
        }
    }

    // Load or generate config.yaml
    let config = AppConfig::load_or_create("config.yaml")?;
    let state = Arc::new(AppState { config: config.clone() });

    // Build Axum router combining API routes and static asset fallback
    let app = Router::new()
        .route("/api/health", get(|| async { "OK" }))
        .merge(routes::create_router(state))
        .fallback(static_handler);

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
