use axum::{http::StatusCode, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;

use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/", get(get_runtime_info))
}

#[derive(Serialize)]
struct RuntimeInfo {
    platform: String,
    latest_version: Option<String>,
    release_url: Option<String>,
}

async fn get_runtime_info() -> Result<Json<RuntimeInfo>, RuntimeInfoError> {
    let platform = format!("{} / {}", platform_name(std::env::consts::OS), architecture_name(std::env::consts::ARCH));
    let response = reqwest::Client::new()
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")
        .header(reqwest::header::USER_AGENT, "Strawberry-Social")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(RuntimeInfoError::upstream)?
        .error_for_status()
        .map_err(RuntimeInfoError::upstream)?;
    let release: serde_json::Value = response.json().await.map_err(RuntimeInfoError::upstream)?;
    let latest = release.get("tag_name").and_then(serde_json::Value::as_str)
        .and_then(|tag| parse_stable_version(tag).map(|_| tag.to_owned()))
        .zip(release.get("html_url").and_then(serde_json::Value::as_str).map(str::to_owned));

    Ok(Json(RuntimeInfo {
        platform,
        latest_version: latest.as_ref().map(|(tag, _)| tag.clone()),
        release_url: latest.map(|(_, url)| url),
    }))
}

fn parse_stable_version(tag: &str) -> Option<(u64, u64, u64)> {
    let version = tag.strip_prefix('v')?;
    let mut parts = version.split('.');
    let parse_part = |part: &str| {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
        part.parse().ok()
    };
    let major = parse_part(parts.next()?)?;
    let minor = parse_part(parts.next()?)?;
    let patch = parse_part(parts.next()?)?;
    if parts.next().is_some() { return None; }
    Some((major, minor, patch))
}

fn platform_name(os: &str) -> &str {
    match os { "macos" => "macOS", "windows" => "Windows", "linux" => "Linux", other => other }
}

fn architecture_name(arch: &str) -> &str {
    match arch { "x86_64" => "x86_64", "aarch64" => "ARM64", other => other }
}

struct RuntimeInfoError(String);
impl RuntimeInfoError {
    fn upstream(error: impl std::fmt::Display) -> Self { Self(error.to_string()) }
}
impl axum::response::IntoResponse for RuntimeInfoError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::BAD_GATEWAY, format!("Could not retrieve llama.cpp releases: {}", self.0)).into_response()
    }
}
