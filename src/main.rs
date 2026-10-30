use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use rust_embed::RustEmbed;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
use config::AppConfig;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Asset;

pub struct AppState {
    pub config: AppConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
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

    // Build Axum router
    let app = Router::new()
        .route("/api/health", get(|| async { "OK" }))
        // Embedded Static Assets
        .fallback(static_handler)
        .with_state(state);

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