use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

use crate::AppState;

const MODEL_NAME: &str = "SmolVLM-256M-Instruct-Q8_0.gguf";
const MODEL_URL: &str = "https://huggingface.co/ggml-org/SmolVLM-256M-Instruct-GGUF/resolve/3e5cd25/SmolVLM-256M-Instruct-Q8_0.gguf";
const MODEL_SHA256: &str = "2a31195d3769c0b0fd0a4906201666108834848db768af11de1d2cef7cd35e65";
const MAX_MODEL_SIZE: u64 = 250 * 1024 * 1024;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(model_status))
        .route("/smolvlm-256m", axum::routing::post(download_model))
}

#[derive(Serialize)]
struct ModelStatus {
    name: &'static str,
    installed: bool,
    size: Option<u64>,
}

async fn model_status(State(state): State<Arc<AppState>>) -> Json<ModelStatus> {
    let path = model_path(&state.data_dir);
    let metadata = tokio::fs::metadata(path).await.ok();
    Json(ModelStatus {
        name: MODEL_NAME,
        installed: metadata.is_some(),
        size: metadata.map(|meta| meta.len()),
    })
}

#[derive(Serialize)]
struct ModelDownloadResponse {
    name: &'static str,
    installed: bool,
    message: String,
}

async fn download_model(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ModelDownloadResponse>, ModelError> {
    let _guard = state.model_download_lock.clone().lock_owned().await;
    let path = model_path(&state.data_dir);
    if tokio::fs::metadata(&path).await.is_ok() {
        return Ok(Json(ModelDownloadResponse {
            name: MODEL_NAME,
            installed: true,
            message: "Model is already downloaded.".into(),
        }));
    }

    let dir = path.parent().ok_or_else(|| {
        ModelError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Models folder is missing.".into(),
        )
    })?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(ModelError::internal)?;
    let temp_path = dir.join(format!(".{MODEL_NAME}.download"));
    let result = download_model_file(&temp_path).await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(error);
    }
    tokio::fs::rename(&temp_path, &path)
        .await
        .map_err(ModelError::internal)?;
    Ok(Json(ModelDownloadResponse {
        name: MODEL_NAME,
        installed: true,
        message: "SmolVLM was downloaded and its SHA-256 checksum verified.".into(),
    }))
}

async fn download_model_file(path: &Path) -> Result<(), ModelError> {
    let response = reqwest::Client::new()
        .get(MODEL_URL)
        .send()
        .await
        .map_err(ModelError::upstream)?
        .error_for_status()
        .map_err(ModelError::upstream)?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MODEL_SIZE)
    {
        return Err(ModelError(
            StatusCode::BAD_GATEWAY,
            "The model file exceeds the 250 MiB safety limit.".into(),
        ));
    }
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(ModelError::internal)?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(ModelError::upstream)?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MAX_MODEL_SIZE {
            return Err(ModelError(
                StatusCode::BAD_GATEWAY,
                "The model download exceeded the 250 MiB safety limit.".into(),
            ));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(ModelError::internal)?;
    }
    file.flush().await.map_err(ModelError::internal)?;
    if downloaded == 0 || format!("{:x}", hasher.finalize()) != MODEL_SHA256 {
        return Err(ModelError(
            StatusCode::BAD_GATEWAY,
            "The model was incomplete or its SHA-256 checksum did not match Hugging Face metadata."
                .into(),
        ));
    }
    Ok(())
}

fn model_path(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join(MODEL_NAME)
}

struct ModelError(StatusCode, String);
impl ModelError {
    fn internal(error: impl std::fmt::Display) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
    fn upstream(error: impl std::fmt::Display) -> Self {
        Self(StatusCode::BAD_GATEWAY, error.to_string())
    }
}
impl IntoResponse for ModelError {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}
