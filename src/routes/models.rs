use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncWriteExt, sync::watch};
use tokio_stream::StreamExt;

use crate::AppState;

const HUB_API: &str = "https://huggingface.co/api";
const HUB_WEB: &str = "https://huggingface.co";
const RESULT_LIMIT: usize = 30;
const PARAMETER_BUCKETS: &[&str] = &[
    "0", "3B", "6B", "9B", "12B", "24B", "32B", "64B", "128B", "256B",
];

static DOWNLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/search", get(search_models))
        .route("/files", get(model_files))
        .route("/installed", get(installed_models))
        .route("/download", post(start_download))
        .route("/download/status", get(download_status))
        .route("/download/control", post(control_download))
        .route("/delete", delete(delete_model))
}

#[derive(Deserialize)]
struct SearchQuery {
    min: Option<String>,
    max: Option<String>,
    search: Option<String>,
}

#[derive(Serialize, Clone)]
struct HubModel {
    repo_id: String,
    url: String,
    parameter_count: Option<u64>,
    parameter_count_estimated: bool,
    last_updated: Option<String>,
    downloads: Option<u64>,
    installed: bool,
}

async fn search_models(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<HubModel>>, ModelError> {
    let range = parameter_range(query.min.as_deref(), query.max.as_deref())?;
    let search = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if search.is_some_and(|value| value.chars().count() > 100) {
        return Err(ModelError(
            StatusCode::BAD_REQUEST,
            "Search text must be 100 characters or fewer.".into(),
        ));
    }
    let trending_url = model_search_url(range.as_deref(), search, "trendingScore");
    let downloads_url = model_search_url(range.as_deref(), search, "downloads");
    let (trending, downloads) = tokio::join!(
        fetch_hub_models(trending_url),
        fetch_hub_models(downloads_url)
    );
    let trending = trending?;
    let downloads = downloads?;
    let installed = installed_repo_ids(&state.data_dir);
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for raw in trending.into_iter().chain(downloads) {
        let Some(model) = parse_hub_model(raw, &installed) else {
            continue;
        };
        // The Hub's web URL supports `num_parameters`, but the REST model list
        // currently accepts the query without applying it. Enforce known counts
        // here as well so the visible results honor the selected range.
        if !matches_parameter_range(
            model.parameter_count,
            query.min.as_deref(),
            query.max.as_deref(),
        ) {
            continue;
        }
        if seen.insert(model.repo_id.clone()) {
            result.push(model);
        }
    }
    Ok(Json(result))
}

fn matches_parameter_range(count: Option<u64>, min: Option<&str>, max: Option<&str>) -> bool {
    let Some(count) = count else {
        // Keep models whose metadata does not publish a count, while showing
        // that limitation explicitly in the UI.
        return true;
    };
    let bound = |value: Option<&str>| {
        value
            .filter(|value| *value != "any")
            .and_then(parse_parameter_count)
    };
    bound(min).is_none_or(|minimum| count >= minimum)
        && bound(max).is_none_or(|maximum| count <= maximum)
}

fn parameter_range(min: Option<&str>, max: Option<&str>) -> Result<Option<String>, ModelError> {
    let min = min.filter(|value| *value != "any");
    let max = max.filter(|value| *value != "any");
    if min.is_some_and(|value| !PARAMETER_BUCKETS.contains(&value))
        || max.is_some_and(|value| !PARAMETER_BUCKETS.contains(&value))
    {
        return Err(ModelError(
            StatusCode::BAD_REQUEST,
            "Choose a parameter bound from the offered Hugging Face ranges.".into(),
        ));
    }
    if let (Some(min), Some(max)) = (min, max) {
        let min_index = PARAMETER_BUCKETS
            .iter()
            .position(|value| *value == min)
            .unwrap();
        let max_index = PARAMETER_BUCKETS
            .iter()
            .position(|value| *value == max)
            .unwrap();
        if min_index > max_index {
            return Err(ModelError(
                StatusCode::BAD_REQUEST,
                "The minimum parameter count must not exceed the maximum.".into(),
            ));
        }
    }
    match (min, max) {
        (Some(min), Some(max)) => Ok(Some(format!("min:{min},max:{max}"))),
        (Some(min), None) => Ok(Some(format!("min:{min}"))),
        (None, Some(max)) => Ok(Some(format!("max:{max}"))),
        (None, None) => Ok(None),
    }
}

fn model_search_url(range: Option<&str>, search: Option<&str>, sort: &str) -> reqwest::Url {
    let mut url = reqwest::Url::parse(&format!("{HUB_API}/models")).expect("valid Hub API URL");
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("pipeline_tag", "text-generation")
            .append_pair("filter", "gguf")
            .append_pair("apps", "llama.cpp")
            .append_pair("sort", sort)
            .append_pair("limit", &RESULT_LIMIT.to_string())
            .append_pair("expand[]", "downloads")
            .append_pair("expand[]", "lastModified")
            .append_pair("expand[]", "tags")
            .append_pair("expand[]", "safetensors")
            .append_pair("expand[]", "config")
            .append_pair("expand[]", "cardData")
            .append_pair("expand[]", "sha");
        if let Some(range) = range {
            query.append_pair("num_parameters", range);
        }
        if let Some(search) = search {
            query.append_pair("search", search);
        }
    }
    url
}

async fn fetch_hub_models(url: reqwest::Url) -> Result<Vec<Value>, ModelError> {
    let response = reqwest::Client::builder()
        .user_agent("Strawberry-Social")
        .build()
        .map_err(ModelError::upstream)?
        .get(url)
        .send()
        .await
        .map_err(ModelError::upstream)?;
    if !response.status().is_success() {
        let status = response.status();
        let message = response
            .headers()
            .get("x-error-message")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Hugging Face returned {status}."));
        return Err(ModelError(StatusCode::BAD_GATEWAY, message));
    }
    response
        .json::<Vec<Value>>()
        .await
        .map_err(ModelError::upstream)
}

fn parse_hub_model(raw: Value, installed: &HashSet<String>) -> Option<HubModel> {
    let repo_id = raw.get("id")?.as_str()?.to_owned();
    if !valid_repo_id(&repo_id) {
        return None;
    }
    let measured_count = raw
        .pointer("/safetensors/parameters/total")
        .and_then(parameter_value)
        .or_else(|| {
            raw.pointer("/config/num_parameters")
                .and_then(parameter_value)
        });
    let declared_count = parameter_count_from_tags(raw.get("tags"));
    let inferred_count = raw
        .pointer("/cardData/base_model")
        .and_then(parameter_count_from_value)
        .or_else(|| parameter_count_from_base_model_tags(raw.get("tags")))
        .or_else(|| parameter_count_from_name(&repo_id));
    let parameter_count = measured_count.or(declared_count).or(inferred_count);
    Some(HubModel {
        url: format!("{HUB_WEB}/{repo_id}"),
        last_updated: raw
            .get("lastModified")
            .and_then(Value::as_str)
            .map(str::to_owned),
        downloads: raw.get("downloads").and_then(Value::as_u64),
        installed: installed.contains(&repo_id),
        repo_id,
        parameter_count,
        parameter_count_estimated: measured_count.is_none()
            && declared_count.is_none()
            && inferred_count.is_some(),
    })
}

fn parameter_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(parse_parameter_count))
}

fn parameter_count_from_tags(tags: Option<&Value>) -> Option<u64> {
    tags?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find_map(|tag| {
            let value = tag
                .strip_prefix("params:")
                .or_else(|| tag.strip_prefix("parameters:"))?;
            parse_parameter_count(value)
        })
}

fn parameter_count_from_base_model_tags(tags: Option<&Value>) -> Option<u64> {
    tags?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|tag| tag.strip_prefix("base_model:"))
        .find_map(parameter_count_from_name)
}

fn parameter_count_from_value(value: &Value) -> Option<u64> {
    if let Some(name) = value.as_str() {
        parameter_count_from_name(name)
    } else if let Some(values) = value.as_array() {
        values.iter().find_map(parameter_count_from_value)
    } else {
        None
    }
}

fn parameter_count_from_name(name: &str) -> Option<u64> {
    let bytes = name.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() || (start > 0 && bytes[start - 1].is_ascii_alphanumeric())
        {
            continue;
        }
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
            end += 1;
        }
        if end < bytes.len()
            && matches!(bytes[end], b'B' | b'b')
            && (end + 1 == bytes.len() || !bytes[end + 1].is_ascii_alphanumeric())
        {
            if let Ok(value) = std::str::from_utf8(&bytes[start..=end]) {
                if let Some(count) = parse_parameter_count(value) {
                    return Some(count);
                }
            }
        }
    }
    None
}

fn parse_parameter_count(value: &str) -> Option<u64> {
    let normalized = value.trim().to_ascii_uppercase();
    let (number, multiplier) = if let Some(number) = normalized.strip_suffix('B') {
        (number, 1_000_000_000u64)
    } else if let Some(number) = normalized.strip_suffix('M') {
        (number, 1_000_000u64)
    } else {
        return normalized.parse().ok();
    };
    let amount = number.parse::<f64>().ok()?;
    if !amount.is_finite() || amount < 0.0 {
        return None;
    }
    Some((amount * multiplier as f64).round() as u64)
}

#[derive(Deserialize)]
struct FilesQuery {
    repo_id: String,
}

#[derive(Serialize, Clone)]
struct ModelFile {
    path: String,
    size: Option<u64>,
    sha256: Option<String>,
    quantization: String,
    kind: &'static str,
}

#[derive(Serialize)]
struct ModelFilesResponse {
    repo_id: String,
    revision: String,
    model_files: Vec<ModelFile>,
    mmproj_files: Vec<ModelFile>,
}

async fn model_files(
    Query(query): Query<FilesQuery>,
) -> Result<Json<ModelFilesResponse>, ModelError> {
    if !valid_repo_id(&query.repo_id) {
        return Err(ModelError(
            StatusCode::BAD_REQUEST,
            "Invalid model repository id.".into(),
        ));
    }
    let mut url = hub_model_info_url(&query.repo_id)?;
    url.query_pairs_mut().append_pair("blobs", "true");
    let info = hub_get_json(url).await?;
    let revision = info
        .get("sha")
        .and_then(Value::as_str)
        .filter(|sha| sha.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(str::to_owned)
        .unwrap_or_else(|| "main".to_owned());
    let mut model_files = Vec::new();
    let mut mmproj_files = Vec::new();
    for sibling in info
        .get("siblings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(path) = sibling.get("rfilename").and_then(Value::as_str) else {
            continue;
        };
        if !path.to_ascii_lowercase().ends_with(".gguf") || !valid_repo_file(path) {
            continue;
        }
        let size = sibling
            .get("size")
            .and_then(Value::as_u64)
            .or_else(|| sibling.pointer("/lfs/size").and_then(Value::as_u64));
        let sha256 = sibling
            .pointer("/lfs/sha256")
            .and_then(Value::as_str)
            .and_then(|oid| oid.strip_prefix("sha256:"))
            .filter(|hash| hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit()))
            .map(str::to_owned);
        let lower = path.to_ascii_lowercase();
        let is_projector = lower.contains("mmproj") || lower.contains("projector");
        let file = ModelFile {
            path: path.to_owned(),
            size,
            sha256,
            quantization: quantization_from_name(path),
            kind: if is_projector { "mmproj" } else { "model" },
        };
        if is_projector {
            mmproj_files.push(file);
        } else {
            model_files.push(file);
        }
    }
    model_files.sort_by(|a, b| a.path.cmp(&b.path));
    mmproj_files.sort_by(|a, b| a.path.cmp(&b.path));
    if model_files.is_empty() {
        return Err(ModelError(
            StatusCode::NOT_FOUND,
            "This repository has no downloadable GGUF model files.".into(),
        ));
    }
    Ok(Json(ModelFilesResponse {
        repo_id: query.repo_id,
        revision,
        model_files,
        mmproj_files,
    }))
}

fn quantization_from_name(path: &str) -> String {
    let upper = path.to_ascii_uppercase();
    for quant in [
        "IQ1_S", "IQ1_M", "IQ2_XXS", "IQ2_XS", "IQ2_S", "IQ2_M", "IQ3_XXS", "IQ3_XS", "IQ3_S",
        "IQ3_M", "IQ4_NL", "IQ4_XS", "Q2_K", "Q3_K_L", "Q3_K_M", "Q3_K_S", "Q4_K_L", "Q4_K_M",
        "Q4_K_S", "Q5_K_L", "Q5_K_M", "Q5_K_S", "Q6_K", "Q8_0", "Q8_K", "Q4_0", "Q4_1", "Q5_0",
        "Q5_1", "Q2_0", "Q3_0", "F64", "F32", "BF16", "F16",
    ] {
        if contains_token(&upper, quant) {
            return quant.to_owned();
        }
    }
    "Unspecified".into()
}

fn contains_token(value: &str, token: &str) -> bool {
    value.match_indices(token).any(|(index, _)| {
        let before = value[..index].chars().next_back();
        let after = value[index + token.len()..].chars().next();
        before.is_none_or(|ch| !ch.is_ascii_alphanumeric())
            && after.is_none_or(|ch| !ch.is_ascii_alphanumeric())
    })
}

#[derive(Deserialize)]
struct StartDownload {
    repo_id: String,
    revision: String,
    model_file: String,
    mmproj_file: Option<String>,
}

#[derive(Serialize)]
struct StartDownloadResponse {
    id: String,
    status: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DownloadControl {
    Running,
    Paused,
    Cancelled,
}

pub struct DownloadJob {
    id: String,
    repo_id: String,
    status: String,
    detail: String,
    current_file: Option<String>,
    downloaded_bytes: u64,
    total_bytes: u64,
    control: watch::Sender<DownloadControl>,
}

#[derive(Serialize)]
struct DownloadStatus {
    id: Option<String>,
    repo_id: Option<String>,
    status: String,
    detail: String,
    current_file: Option<String>,
    downloaded_bytes: u64,
    total_bytes: u64,
}

async fn start_download(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartDownload>,
) -> Result<Json<StartDownloadResponse>, ModelError> {
    if !valid_repo_id(&request.repo_id)
        || !valid_revision(&request.revision)
        || !valid_repo_file(&request.model_file)
        || request
            .mmproj_file
            .as_deref()
            .is_some_and(|path| !valid_repo_file(path))
    {
        return Err(ModelError(
            StatusCode::BAD_REQUEST,
            "Invalid model download selection.".into(),
        ));
    }
    let files = fetch_selected_files(&request).await?;
    let dir = model_directory(&state.data_dir, &request.repo_id)?;
    let mut selected = vec![files.0];
    if let Some(projector) = files.1 {
        selected.push(projector);
    }
    let total_bytes = selected
        .iter()
        .filter(|file| !dir.join(&file.path).is_file())
        .filter_map(|file| file.size)
        .sum();

    let id = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        DOWNLOAD_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let (control, receiver) = watch::channel(DownloadControl::Running);
    {
        let mut job = state.model_download.lock().await;
        if job
            .as_ref()
            .is_some_and(|job| is_download_active(&job.status))
        {
            return Err(ModelError(
                StatusCode::CONFLICT,
                "A model download is already active. Finish or cancel it before starting another."
                    .into(),
            ));
        }
        *job = Some(DownloadJob {
            id: id.clone(),
            repo_id: request.repo_id.clone(),
            status: "downloading".into(),
            detail: "Preparing the selected model files.".into(),
            current_file: None,
            downloaded_bytes: 0,
            total_bytes,
            control: control.clone(),
        });
    }
    let app_state = state.clone();
    let task_id = id.clone();
    tokio::spawn(async move {
        run_download(app_state, task_id, request, dir, selected, receiver).await;
    });
    Ok(Json(StartDownloadResponse {
        id,
        status: "downloading",
    }))
}

async fn fetch_selected_files(
    request: &StartDownload,
) -> Result<(ModelFile, Option<ModelFile>), ModelError> {
    let mut url = hub_model_info_url(&request.repo_id)?;
    url.query_pairs_mut().append_pair("blobs", "true");
    let info = hub_get_json(url).await?;
    let selected = info
        .get("siblings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let path = item.get("rfilename")?.as_str()?;
            if !path.to_ascii_lowercase().ends_with(".gguf") || !valid_repo_file(path) {
                return None;
            }
            Some(ModelFile {
                path: path.into(),
                size: item
                    .get("size")
                    .and_then(Value::as_u64)
                    .or_else(|| item.pointer("/lfs/size").and_then(Value::as_u64)),
                sha256: item
                    .pointer("/lfs/sha256")
                    .and_then(Value::as_str)
                    .and_then(|oid| oid.strip_prefix("sha256:"))
                    .filter(|hash| {
                        hash.len() == 64 && hash.chars().all(|ch| ch.is_ascii_hexdigit())
                    })
                    .map(str::to_owned),
                quantization: quantization_from_name(path),
                kind: if path.to_ascii_lowercase().contains("mmproj")
                    || path.to_ascii_lowercase().contains("projector")
                {
                    "mmproj"
                } else {
                    "model"
                },
            })
        })
        .collect::<Vec<_>>();
    let model = selected
        .iter()
        .find(|file| file.path == request.model_file && file.kind == "model")
        .cloned()
        .ok_or_else(|| {
            ModelError(
                StatusCode::BAD_REQUEST,
                "The selected model file is not in this repository.".into(),
            )
        })?;
    let projector = if let Some(path) = &request.mmproj_file {
        Some(
            selected
                .iter()
                .find(|file| file.path == *path && file.kind == "mmproj")
                .cloned()
                .ok_or_else(|| {
                    ModelError(
                        StatusCode::BAD_REQUEST,
                        "The selected mmproj file is not in this repository.".into(),
                    )
                })?,
        )
    } else {
        None
    };
    Ok((model, projector))
}

async fn run_download(
    state: Arc<AppState>,
    id: String,
    request: StartDownload,
    directory: PathBuf,
    files: Vec<ModelFile>,
    mut control: watch::Receiver<DownloadControl>,
) {
    let mut created_files = Vec::new();
    let result = async {
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(ModelError::internal)?;
        for file in &files {
            if wait_for_download_control(&mut control).await {
                return Err(ModelError(
                    StatusCode::CONFLICT,
                    "Download cancelled.".into(),
                ));
            }
            set_job_file(&state, &id, &file.path).await;
            let output = directory.join(&file.path);
            if output.is_file() {
                continue;
            }
            if let Some(parent) = output.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(ModelError::internal)?;
            }
            let temp = output.with_file_name(format!(
                ".{}.{}.part",
                output
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("model.gguf"),
                id
            ));
            let url = resolve_file_url(&request.repo_id, &request.revision, &file.path)?;
            if let Err(error) = download_one(
                &state,
                &id,
                &url,
                &temp,
                file.size,
                file.sha256.as_deref(),
                &mut control,
            )
            .await
            {
                let _ = tokio::fs::remove_file(&temp).await;
                return Err(error);
            }
            if wait_for_download_control(&mut control).await {
                let _ = tokio::fs::remove_file(&temp).await;
                return Err(ModelError(
                    StatusCode::CONFLICT,
                    "Download cancelled.".into(),
                ));
            }
            tokio::fs::rename(&temp, &output)
                .await
                .map_err(ModelError::internal)?;
            created_files.push(output);
        }
        if wait_for_download_control(&mut control).await {
            return Err(ModelError(
                StatusCode::CONFLICT,
                "Download cancelled.".into(),
            ));
        }
        write_model_manifest(&directory, &request, &files).await?;
        Ok::<(), ModelError>(())
    }
    .await;

    let (status, detail) = match result {
        Ok(()) => (
            "completed",
            "Selected files are downloaded and ready.".to_owned(),
        ),
        Err(error) if error.1 == "Download cancelled." => {
            for path in created_files {
                let _ = tokio::fs::remove_file(path).await;
            }
            (
                "cancelled",
                "The download was cancelled and partial files were removed.".to_owned(),
            )
        }
        Err(error) => {
            for path in created_files {
                let _ = tokio::fs::remove_file(path).await;
            }
            ("failed", error.1)
        }
    };
    let mut job = state.model_download.lock().await;
    if let Some(job) = job.as_mut().filter(|job| job.id == id) {
        job.status = status.into();
        job.detail = detail;
        job.current_file = None;
        job.control.send_replace(DownloadControl::Running);
    }
}

async fn download_one(
    state: &AppState,
    id: &str,
    url: &reqwest::Url,
    destination: &Path,
    expected_size: Option<u64>,
    expected_sha256: Option<&str>,
    control: &mut watch::Receiver<DownloadControl>,
) -> Result<(), ModelError> {
    let response = reqwest::Client::builder()
        .user_agent("Strawberry-Social")
        .build()
        .map_err(ModelError::upstream)?
        .get(url.clone())
        .send()
        .await
        .map_err(ModelError::upstream)?
        .error_for_status()
        .map_err(ModelError::upstream)?;
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(ModelError::internal)?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = stream.next().await {
        if wait_for_download_control(control).await {
            drop(file);
            let _ = tokio::fs::remove_file(destination).await;
            return Err(ModelError(
                StatusCode::CONFLICT,
                "Download cancelled.".into(),
            ));
        }
        let chunk = chunk.map_err(ModelError::upstream)?;
        file.write_all(&chunk).await.map_err(ModelError::internal)?;
        hasher.update(&chunk);
        add_download_progress(state, id, chunk.len() as u64).await;
    }
    file.flush().await.map_err(ModelError::internal)?;
    let actual_size = file.metadata().await.map_err(ModelError::internal)?.len();
    if actual_size == 0 || expected_size.is_some_and(|expected| expected != actual_size) {
        return Err(ModelError(
            StatusCode::BAD_GATEWAY,
            format!(
                "Downloaded file size was {actual_size} bytes; expected {}.",
                expected_size.unwrap_or_default()
            ),
        ));
    }
    if expected_sha256.is_some_and(|expected| format!("{:x}", hasher.finalize()) != expected) {
        return Err(ModelError(
            StatusCode::BAD_GATEWAY,
            "The downloaded model file did not match Hugging Face's SHA-256 checksum.".into(),
        ));
    }
    Ok(())
}

async fn wait_for_download_control(control: &mut watch::Receiver<DownloadControl>) -> bool {
    loop {
        let current = { *control.borrow_and_update() };
        match current {
            DownloadControl::Running => return false,
            DownloadControl::Cancelled => return true,
            DownloadControl::Paused => {
                if control.changed().await.is_err() {
                    return true;
                }
            }
        }
    }
}

async fn set_job_file(state: &AppState, id: &str, path: &str) {
    if let Some(job) = state
        .model_download
        .lock()
        .await
        .as_mut()
        .filter(|job| job.id == id)
    {
        job.current_file = Some(path.to_owned());
        job.detail = format!("Downloading {path}");
    }
}

async fn add_download_progress(state: &AppState, id: &str, bytes: u64) {
    if let Some(job) = state
        .model_download
        .lock()
        .await
        .as_mut()
        .filter(|job| job.id == id)
    {
        job.downloaded_bytes = job.downloaded_bytes.saturating_add(bytes);
    }
}

async fn write_model_manifest(
    directory: &Path,
    request: &StartDownload,
    files: &[ModelFile],
) -> Result<(), ModelError> {
    let manifest = serde_json::json!({
        "repo_id": request.repo_id,
        "revision": request.revision,
        "model_file": request.model_file,
        "mmproj_file": request.mmproj_file,
        "downloaded_files": files.iter().map(|file| &file.path).collect::<Vec<_>>(),
    });
    tokio::fs::write(
        directory.join(".strawberry-model.json"),
        serde_json::to_vec_pretty(&manifest).map_err(ModelError::internal)?,
    )
    .await
    .map_err(ModelError::internal)
}

#[derive(Deserialize)]
struct DownloadControlRequest {
    id: String,
    action: String,
}

async fn control_download(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DownloadControlRequest>,
) -> Result<Json<DownloadStatus>, ModelError> {
    let mut active = state.model_download.lock().await;
    let job = active
        .as_mut()
        .filter(|job| job.id == request.id)
        .ok_or_else(|| ModelError(StatusCode::NOT_FOUND, "Download job was not found.".into()))?;
    if !is_download_active(&job.status) {
        return Err(ModelError(
            StatusCode::CONFLICT,
            "This download has already finished.".into(),
        ));
    }
    match request.action.as_str() {
        "pause" => {
            job.status = "paused".into();
            job.detail = "Download paused.".into();
            job.control.send_replace(DownloadControl::Paused);
        }
        "resume" => {
            job.status = "downloading".into();
            job.detail = "Download resumed.".into();
            job.control.send_replace(DownloadControl::Running);
        }
        "cancel" => {
            job.status = "cancelling".into();
            job.detail = "Cancelling download…".into();
            job.control.send_replace(DownloadControl::Cancelled);
        }
        _ => {
            return Err(ModelError(
                StatusCode::BAD_REQUEST,
                "Action must be pause, resume, or cancel.".into(),
            ))
        }
    }
    Ok(Json(download_snapshot(Some(job))))
}

async fn download_status(State(state): State<Arc<AppState>>) -> Json<DownloadStatus> {
    let active = state.model_download.lock().await;
    Json(download_snapshot(active.as_ref()))
}

fn download_snapshot(job: Option<&DownloadJob>) -> DownloadStatus {
    DownloadStatus {
        id: job.map(|job| job.id.clone()),
        repo_id: job.map(|job| job.repo_id.clone()),
        status: job
            .map(|job| job.status.clone())
            .unwrap_or_else(|| "idle".into()),
        detail: job.map(|job| job.detail.clone()).unwrap_or_default(),
        current_file: job.and_then(|job| job.current_file.clone()),
        downloaded_bytes: job.map(|job| job.downloaded_bytes).unwrap_or_default(),
        total_bytes: job.map(|job| job.total_bytes).unwrap_or_default(),
    }
}

fn is_download_active(status: &str) -> bool {
    matches!(status, "downloading" | "paused" | "cancelling")
}

#[derive(Deserialize)]
struct DeleteModelRequest {
    repo_id: String,
}

#[derive(Serialize)]
struct DeleteModelResponse {
    deleted: bool,
    repo_id: String,
}

async fn delete_model(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DeleteModelRequest>,
) -> Result<Json<DeleteModelResponse>, ModelError> {
    if !valid_repo_id(&request.repo_id) {
        return Err(ModelError(
            StatusCode::BAD_REQUEST,
            "Invalid model repository id.".into(),
        ));
    }
    let _process = state.runtime_server.lock().await;
    let running = state.active_model.lock().await;
    if _process.is_none() && super::runtime_server::external_server_health(&state).await {
        return Err(ModelError(
            StatusCode::CONFLICT,
            "A server is responding at the configured API address, but Strawberry Social cannot tell which model it loaded. Stop that server before deleting downloaded models.".into(),
        ));
    }
    if running
        .as_ref()
        .is_some_and(|model| model.repo_id == request.repo_id)
    {
        return Err(ModelError(
            StatusCode::CONFLICT,
            "This model is currently loaded. Stop llama.cpp or switch to another model before deleting it.".into(),
        ));
    }
    let directory = model_directory(&state.data_dir, &request.repo_id)?;
    let directory_type = tokio::fs::symlink_metadata(&directory)
        .await
        .map_err(ModelError::internal)?
        .file_type();
    if !directory_type.is_dir() || directory_type.is_symlink() {
        return Err(ModelError(
            StatusCode::NOT_FOUND,
            "This model is not installed.".into(),
        ));
    }
    let models_root = state
        .data_dir
        .join("models")
        .canonicalize()
        .map_err(ModelError::internal)?;
    let canonical = directory.canonicalize().map_err(ModelError::internal)?;
    if !canonical.starts_with(&models_root) || canonical == models_root {
        return Err(ModelError(
            StatusCode::BAD_REQUEST,
            "Model folder path is invalid.".into(),
        ));
    }
    tokio::fs::remove_dir_all(directory)
        .await
        .map_err(ModelError::internal)?;
    Ok(Json(DeleteModelResponse {
        deleted: true,
        repo_id: request.repo_id,
    }))
}

#[derive(Serialize)]
struct InstalledModel {
    repo_id: String,
    files: Vec<String>,
    model_file: Option<String>,
    mmproj_file: Option<String>,
    active: bool,
    size: u64,
}

async fn installed_models(State(state): State<Arc<AppState>>) -> Json<Vec<InstalledModel>> {
    let active = state.active_model.lock().await.clone();
    Json(scan_installed_models(&state.data_dir, active.as_ref()))
}

fn scan_installed_models(
    data_dir: &Path,
    active_model: Option<&crate::ActiveModel>,
) -> Vec<InstalledModel> {
    let root = data_dir.join("models");
    let mut models = Vec::new();
    let Ok(owners) = std::fs::read_dir(&root) else {
        return models;
    };
    for owner in owners
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    {
        let Ok(repos) = std::fs::read_dir(owner.path()) else {
            continue;
        };
        for repo in repos
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        {
            let repo_id = format!(
                "{}/{}",
                owner.file_name().to_string_lossy(),
                repo.file_name().to_string_lossy()
            );
            if !valid_repo_id(&repo_id) {
                continue;
            }
            let mut files = Vec::new();
            let mut size = 0u64;
            collect_gguf_files(&repo.path(), &repo.path(), &mut files, &mut size);
            if files.is_empty() {
                continue;
            }
            files.sort();
            let manifest = std::fs::read(repo.path().join(".strawberry-model.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            let model_file = active_model
                .filter(|model| model.repo_id == repo_id)
                .map(|model| model.model_file.clone())
                .filter(|file| files.contains(file))
                .or_else(|| {
                    manifest
                        .as_ref()
                        .and_then(|m| m.get("model_file"))
                        .and_then(Value::as_str)
                        .filter(|file| files.contains(&file.to_string()))
                        .map(str::to_owned)
                        .or_else(|| {
                            files
                                .iter()
                                .find(|file| !file.to_ascii_lowercase().contains("mmproj"))
                                .cloned()
                        })
                });
            let mmproj_file = manifest
                .as_ref()
                .and_then(|m| m.get("mmproj_file"))
                .and_then(Value::as_str)
                .filter(|file| files.contains(&file.to_string()))
                .map(str::to_owned)
                .or_else(|| {
                    files
                        .iter()
                        .find(|file| file.to_ascii_lowercase().contains("mmproj"))
                        .cloned()
                });
            models.push(InstalledModel {
                active: active_model.is_some_and(|model| model.repo_id == repo_id),
                repo_id,
                files,
                model_file,
                mmproj_file,
                size,
            });
        }
    }
    models.sort_by(|a, b| a.repo_id.cmp(&b.repo_id));
    models
}

fn collect_gguf_files(root: &Path, directory: &Path, files: &mut Vec<String>, total: &mut u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            collect_gguf_files(root, &path, files, total);
        } else if entry.file_type().is_ok_and(|kind| kind.is_file())
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
        {
            if let Ok(relative) = path.strip_prefix(root) {
                if let Ok(metadata) = entry.metadata() {
                    *total = total.saturating_add(metadata.len());
                }
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

pub(crate) fn migrate_legacy_model(data_dir: &Path) -> std::io::Result<()> {
    let old_path = data_dir.join("models/SmolVLM-256M-Instruct-Q8_0.gguf");
    if !old_path.is_file() {
        return Ok(());
    }
    let directory = data_dir.join("models/ggml-org/SmolVLM-256M-Instruct-GGUF");
    std::fs::create_dir_all(&directory)?;
    let new_path = directory.join("SmolVLM-256M-Instruct-Q8_0.gguf");
    if !new_path.exists() {
        std::fs::rename(old_path, &new_path)?;
    }
    let manifest = serde_json::json!({
        "repo_id": "ggml-org/SmolVLM-256M-Instruct-GGUF",
        "revision": "3e5cd25",
        "model_file": "SmolVLM-256M-Instruct-Q8_0.gguf",
        "mmproj_file": null,
        "downloaded_files": ["SmolVLM-256M-Instruct-Q8_0.gguf"],
    });
    std::fs::write(
        directory.join(".strawberry-model.json"),
        serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    )?;
    Ok(())
}

pub(crate) fn cleanup_incomplete_downloads(data_dir: &Path) {
    fn clean(directory: &Path) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                clean(&path);
            } else if kind.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".part"))
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    clean(&data_dir.join("models"));
}

fn installed_repo_ids(data_dir: &Path) -> HashSet<String> {
    scan_installed_models(data_dir, None)
        .into_iter()
        .map(|model| model.repo_id)
        .collect()
}

fn model_directory(data_dir: &Path, repo_id: &str) -> Result<PathBuf, ModelError> {
    if !valid_repo_id(repo_id) {
        return Err(ModelError(
            StatusCode::BAD_REQUEST,
            "Invalid model repository id.".into(),
        ));
    }
    let mut components = repo_id.split('/');
    let owner = components.next().unwrap_or_default();
    let repo = components.next().unwrap_or_default();
    Ok(data_dir.join("models").join(owner).join(repo))
}

fn valid_repo_id(repo_id: &str) -> bool {
    let mut parts = repo_id.split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some(owner), Some(repo), None)
        if valid_path_segment(owner) && valid_path_segment(repo))
}

fn valid_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn valid_repo_file(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

pub(crate) fn resolve_installed_file(
    data_dir: &Path,
    repo_id: &str,
    file: &str,
) -> Result<PathBuf, String> {
    if !valid_repo_file(file) || !file.to_ascii_lowercase().ends_with(".gguf") {
        return Err("Choose a GGUF file from the selected repository.".into());
    }
    let directory = model_directory(data_dir, repo_id).map_err(|error| error.1)?;
    let root = directory
        .canonicalize()
        .map_err(|_| "This model is not installed.".to_owned())?;
    let path = directory
        .join(file)
        .canonicalize()
        .map_err(|_| "The selected model file is not installed.".to_owned())?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err("The selected model file path is invalid.".into());
    }
    Ok(path)
}

fn valid_revision(revision: &str) -> bool {
    !revision.is_empty()
        && revision.len() <= 128
        && revision
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn hub_model_info_url(repo_id: &str) -> Result<reqwest::Url, ModelError> {
    let url = format!("{HUB_API}/models/{repo_id}");
    reqwest::Url::parse(&url).map_err(ModelError::upstream)
}

async fn hub_get_json(url: reqwest::Url) -> Result<Value, ModelError> {
    reqwest::Client::builder()
        .user_agent("Strawberry-Social")
        .build()
        .map_err(ModelError::upstream)?
        .get(url)
        .send()
        .await
        .map_err(ModelError::upstream)?
        .error_for_status()
        .map_err(ModelError::upstream)?
        .json()
        .await
        .map_err(ModelError::upstream)
}

fn resolve_file_url(repo_id: &str, revision: &str, file: &str) -> Result<reqwest::Url, ModelError> {
    let mut url = reqwest::Url::parse(&format!("{HUB_WEB}/{repo_id}/resolve/{revision}"))
        .map_err(ModelError::upstream)?;
    url.path_segments_mut()
        .map_err(|_| ModelError::internal("invalid Hugging Face file URL"))?
        .extend(file.split('/'));
    Ok(url)
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
