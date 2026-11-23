use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
#[cfg(target_os = "linux")]
use directories::BaseDirs;
#[cfg(not(target_os = "linux"))]
use directories::ProjectDirs;
use rust_embed::RustEmbed;
use std::fs::File;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod card;
mod config;
mod db;
mod llm;
mod routes;

use config::AppConfig;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Asset;

pub struct AppState {
    pub config: AppConfig,
    pub generation: Arc<tokio::sync::RwLock<config::GenerationSettings>>,
    pub data_dir: PathBuf,
    pub runtime_download_lock: Arc<tokio::sync::Mutex<()>>,
    pub model_download: Arc<tokio::sync::Mutex<Option<routes::models::DownloadJob>>>,
    pub runtime_server: Arc<tokio::sync::Mutex<Option<tokio::process::Child>>>,
    pub active_model: Arc<tokio::sync::Mutex<Option<ActiveModel>>>,
}

#[derive(Clone, serde::Serialize)]
pub struct ActiveModel {
    pub repo_id: String,
    pub model_file: String,
    pub mmproj_file: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Determine platform-standard data directory
    // Linux: ~/.local/share/strawberry-social
    // Windows: C:\Users\<User>\AppData\Local\StrawberrySocial
    #[cfg(target_os = "linux")]
    let (data_dir, legacy_data_dir) = {
        let base_dirs = BaseDirs::new().ok_or("Failed to determine home directory")?;
        let data_root = base_dirs.data_local_dir();
        (
            data_root.join("strawberry-social"),
            Some(data_root.join("strawberrysocial")),
        )
    };
    #[cfg(not(target_os = "linux"))]
    let (data_dir, legacy_data_dir) = {
        let proj_dirs = ProjectDirs::from("com", "StrawberrySocial", "StrawberrySocial")
            .ok_or("Failed to determine home directory")?;
        (proj_dirs.data_local_dir().to_path_buf(), None::<PathBuf>)
    };

    if let Some(legacy_dir) = legacy_data_dir.as_deref() {
        migrate_missing_data(legacy_dir, &data_dir)?;
    }
    std::fs::create_dir_all(&data_dir)?;
    routes::models::migrate_legacy_model(&data_dir)?;
    routes::models::cleanup_incomplete_downloads(&data_dir);

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
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(log_file)
                .with_ansi(false),
        )
        .init();

    // Load or generate config.yaml in the standard directory
    let config = AppConfig::load_or_create(&config_path)?;
    let state = Arc::new(AppState {
        config: config.clone(),
        generation: Arc::new(tokio::sync::RwLock::new((&config.llm).into())),
        data_dir: data_dir.to_path_buf(),
        runtime_download_lock: Arc::new(tokio::sync::Mutex::new(())),
        model_download: Arc::new(tokio::sync::Mutex::new(None)),
        runtime_server: Arc::new(tokio::sync::Mutex::new(None)),
        active_model: Arc::new(tokio::sync::Mutex::new(None)),
    });

    // Build Axum router combining API routes, config, and static asset fallback
    let app = Router::new()
        .route("/api/health", get(|| async { "OK" }))
        .merge(routes::create_router(state.clone()))
        .fallback(static_handler)
        .with_state(state.clone());

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    tracing::info!("Strawberry Social starting on http://{}", addr);

    if config.server.open_browser_on_start && std::env::var_os("STRAWBERRY_NO_BROWSER").is_none() {
        let _ = open::that(format!("http://{}", addr));
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn migrate_missing_data(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<()> {
    if !source.is_dir() || source == destination {
        return Ok(());
    }

    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            migrate_missing_data(&source_path, &destination_path)?;
        } else if file_type.is_file() && !destination_path.exists() {
            if let Some(parent) = destination_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&source_path, &destination_path)?;
        }
    }
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
                .header(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(mime.as_ref()).unwrap(),
                )
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
