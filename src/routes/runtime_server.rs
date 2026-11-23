use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    net::IpAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::process::Command;

use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(get_status))
        .route("/start", axum::routing::post(start_server))
        .route("/stop", axum::routing::post(stop_server))
}

#[derive(Serialize)]
struct ServerStatus {
    state: &'static str,
    detail: String,
    managed: bool,
    active_model: Option<crate::ActiveModel>,
}

async fn get_status(State(state): State<Arc<AppState>>) -> Json<ServerStatus> {
    let mut process = state.runtime_server.lock().await;
    let Some(child) = process.as_mut() else {
        let (_, _, health_url) = server_address(&state)
            .unwrap_or_else(|_| (String::new(), String::new(), String::new()));
        if !health_url.is_empty() && health_check(&health_url).await {
            return Json(ServerStatus {
                state: "running",
                detail:
                    "A server is responding at the configured API address (externally managed)."
                        .into(),
                managed: false,
                active_model: None,
            });
        }
        return Json(ServerStatus {
            state: "offline",
            detail: "The managed llama.cpp server is stopped.".into(),
            managed: false,
            active_model: None,
        });
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            *process = None;
            *state.active_model.lock().await = None;
            Json(ServerStatus {
                state: "offline",
                detail: format!("llama-server exited with {status}."),
                managed: false,
                active_model: None,
            })
        }
        Ok(None) => {
            let (_, _, health_url) = server_address(&state)
                .unwrap_or_else(|_| (String::new(), String::new(), String::new()));
            if health_url.is_empty() || !health_check(&health_url).await {
                Json(ServerStatus {
                    state: "starting",
                    detail: "llama-server is loading the model.".into(),
                    managed: true,
                    active_model: state.active_model.lock().await.clone(),
                })
            } else {
                Json(ServerStatus {
                    state: "running",
                    detail: "The model is loaded and ready.".into(),
                    managed: true,
                    active_model: state.active_model.lock().await.clone(),
                })
            }
        }
        Err(error) => Json(ServerStatus {
            state: "offline",
            detail: format!("Could not inspect llama-server: {error}"),
            managed: false,
            active_model: None,
        }),
    }
}

#[derive(Serialize)]
struct ServerActionResponse {
    state: &'static str,
    detail: String,
}

#[derive(Deserialize)]
struct StartServerRequest {
    repo_id: String,
    model_file: String,
    mmproj_file: Option<String>,
}

async fn start_server(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartServerRequest>,
) -> Result<Json<ServerActionResponse>, ServerError> {
    if !cfg!(target_os = "linux") {
        return Err(ServerError(
            StatusCode::NOT_IMPLEMENTED,
            "The managed llama.cpp server is currently supported on Linux only.".into(),
        ));
    }
    let mut process = state.runtime_server.lock().await;
    let (_, _, configured_health_url) = server_address(&state)?;
    if let Some(child) = process.as_mut() {
        if child.try_wait().map_err(ServerError::internal)?.is_none() {
            let active = state.active_model.lock().await.clone();
            if active.as_ref().is_some_and(|model| {
                model.repo_id != request.repo_id
                    || model.model_file != request.model_file
                    || model.mmproj_file != request.mmproj_file
            }) {
                return Err(ServerError(
                    StatusCode::CONFLICT,
                    "Another model is currently loaded. Stop llama.cpp before switching models."
                        .into(),
                ));
            }
            if health_check(&configured_health_url).await {
                return Ok(Json(ServerActionResponse {
                    state: "running",
                    detail: "The model is loaded and ready.".into(),
                }));
            }
            return Ok(Json(ServerActionResponse {
                state: "starting",
                detail: "llama-server is already loading the model.".into(),
            }));
        }
        *process = None;
        *state.active_model.lock().await = None;
    }
    if health_check(&configured_health_url).await {
        return Ok(Json(ServerActionResponse {
            state: "running",
            detail: "A server is already responding at the configured API address. Strawberry Social left it untouched.".into(),
        }));
    }

    let model_path = super::models::resolve_installed_file(
        &state.data_dir,
        &request.repo_id,
        &request.model_file,
    )
    .map_err(|message| ServerError(StatusCode::CONFLICT, message))?;
    let mmproj_path = request
        .mmproj_file
        .as_deref()
        .map(|file| {
            super::models::resolve_installed_file(&state.data_dir, &request.repo_id, file)
                .map_err(|message| ServerError(StatusCode::CONFLICT, message))
        })
        .transpose()?;
    let server_path = installed_server_path(&state.data_dir)?;
    let (host, port, _) = server_address(&state)?;
    let log_path = state.data_dir.join("llama-server.log");
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(ServerError::internal)?;
    let stdout = log.try_clone().map_err(ServerError::internal)?;

    let mut command = Command::new(&server_path);
    command.arg("--model").arg(&model_path);
    if let Some(mmproj_path) = &mmproj_path {
        command.arg("--mmproj").arg(mmproj_path);
    }
    command
        .arg("--host")
        .arg(&host)
        .arg("--port")
        .arg(port.to_string())
        .arg("--ctx-size")
        .arg(state.config.llm.context_window.to_string())
        .arg("--alias")
        .arg(&state.config.llm.default_model)
        .current_dir(server_path.parent().unwrap_or(Path::new(".")))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(log))
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    if let Some(parent) = server_path.parent() {
        command.env("LD_LIBRARY_PATH", parent);
    }

    let child = command.spawn().map_err(|error| {
        ServerError(
            StatusCode::BAD_GATEWAY,
            format!("Could not launch llama-server: {error}. See llama-server.log for details."),
        )
    })?;
    *process = Some(child);
    *state.active_model.lock().await = Some(crate::ActiveModel {
        repo_id: request.repo_id,
        model_file: request.model_file,
        mmproj_file: request.mmproj_file,
    });
    Ok(Json(ServerActionResponse {
        state: "starting",
        detail: "llama-server started and is loading the selected model.".into(),
    }))
}

async fn stop_server(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ServerActionResponse>, ServerError> {
    let mut process = state.runtime_server.lock().await;
    if let Some(mut child) = process.take() {
        if child.try_wait().map_err(ServerError::internal)?.is_none() {
            child.kill().await.map_err(ServerError::internal)?;
        }
        let _ = child.wait().await;
        *state.active_model.lock().await = None;
    }
    Ok(Json(ServerActionResponse {
        state: "offline",
        detail: "The managed llama.cpp server is stopped.".into(),
    }))
}

fn installed_server_path(data_dir: &Path) -> Result<PathBuf, ServerError> {
    let runtime_root = data_dir.join("llama.cpp");
    let marker = fs::read(runtime_root.join("runtime.json")).map_err(|_| {
        ServerError(
            StatusCode::CONFLICT,
            "Install the llama.cpp runtime before starting the server.".into(),
        )
    })?;
    let metadata: serde_json::Value =
        serde_json::from_slice(&marker).map_err(ServerError::internal)?;
    let relative = metadata
        .get("server_path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ServerError(
                StatusCode::CONFLICT,
                "The installed llama.cpp runtime has no server binary path.".into(),
            )
        })?;
    let root = runtime_root.canonicalize().map_err(ServerError::internal)?;
    let server = runtime_root
        .join(relative)
        .canonicalize()
        .map_err(ServerError::internal)?;
    if !server.starts_with(&root) || !server.is_file() {
        return Err(ServerError(
            StatusCode::CONFLICT,
            "The installed llama-server path is invalid.".into(),
        ));
    }
    super::runtime_info::ensure_library_soname_links(&server).map_err(ServerError::internal)?;
    Ok(server)
}

fn server_address(state: &AppState) -> Result<(String, String, String), ServerError> {
    let url = reqwest::Url::parse(&state.config.llm.api_base_url).map_err(|error| {
        ServerError(
            StatusCode::BAD_REQUEST,
            format!("Invalid configured API URL: {error}"),
        )
    })?;
    let host = url.host_str().ok_or_else(|| {
        ServerError(
            StatusCode::BAD_REQUEST,
            "The configured API URL has no host.".into(),
        )
    })?;
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !is_loopback {
        return Err(ServerError(StatusCode::BAD_REQUEST, "Built-in llama.cpp can only bind to a loopback API URL. Update llm.api_base_url in config.yaml to use localhost or 127.0.0.1.".into()));
    }
    let port = url.port_or_known_default().ok_or_else(|| {
        ServerError(
            StatusCode::BAD_REQUEST,
            "The configured API URL has no port.".into(),
        )
    })?;
    let host = "127.0.0.1".to_string();
    let base = format!("http://{host}:{port}");
    Ok((host, port.to_string(), format!("{base}/health")))
}

async fn health_check(url: &str) -> bool {
    reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_millis(800))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

pub(crate) async fn external_server_health(state: &AppState) -> bool {
    if let Ok((_, _, url)) = server_address(state) {
        health_check(&url).await
    } else {
        false
    }
}

struct ServerError(StatusCode, String);
impl ServerError {
    fn internal(error: impl std::fmt::Display) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}
impl IntoResponse for ServerError {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}
