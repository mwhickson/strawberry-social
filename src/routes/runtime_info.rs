use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
#[cfg(not(target_os = "windows"))]
use flate2::read::GzDecoder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(not(target_os = "windows"))]
use tar::Archive;
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

use crate::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(get_runtime_info))
        .route("/download", axum::routing::post(download_runtime))
}

#[derive(Serialize)]
struct RuntimeInfo {
    platform: String,
    latest_version: Option<String>,
    release_url: Option<String>,
    runtime_tag: Option<String>,
    asset_name: Option<String>,
    asset_size: Option<u64>,
    installed_runtime_tag: Option<String>,
    auto_download_runtime: bool,
}

#[derive(Clone)]
struct RuntimeAsset {
    stable_version: String,
    stable_url: String,
    runtime_tag: String,
    name: String,
    url: String,
    size: u64,
    sha256: String,
}

async fn get_runtime_info(
    State(state): State<Arc<AppState>>,
) -> Result<Json<RuntimeInfo>, RuntimeError> {
    let platform = platform_label();
    if !is_supported_cpu_target() {
        let (stable_version, stable_url, nightly_tag) = resolve_stable_release().await?;
        return Ok(Json(RuntimeInfo {
            platform,
            latest_version: Some(stable_version),
            release_url: Some(stable_url),
            runtime_tag: Some(nightly_tag),
            asset_name: None,
            asset_size: None,
            installed_runtime_tag: read_installed_runtime_tag(&state.data_dir),
            auto_download_runtime: read_auto_download_preference(&state.data_dir),
        }));
    }
    let asset = resolve_cpu_asset().await?;
    Ok(Json(RuntimeInfo {
        platform,
        latest_version: Some(asset.stable_version),
        release_url: Some(asset.stable_url),
        runtime_tag: Some(asset.runtime_tag),
        asset_name: Some(asset.name),
        asset_size: Some(asset.size),
        installed_runtime_tag: read_installed_runtime_tag(&state.data_dir),
        auto_download_runtime: read_auto_download_preference(&state.data_dir),
    }))
}

#[derive(Serialize)]
struct DownloadResponse {
    runtime_tag: String,
    binary_verified: bool,
    message: String,
}

async fn download_runtime(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DownloadResponse>, RuntimeError> {
    if !is_supported_cpu_target() {
        return Err(RuntimeError(
            StatusCode::NOT_IMPLEMENTED,
            format!(
                "Built-in runtime downloads are not available for {} yet.",
                platform_label()
            ),
        ));
    }

    let _guard = state.runtime_download_lock.clone().lock_owned().await;
    let asset = resolve_cpu_asset().await?;
    let root = state.data_dir.join("llama.cpp");
    let versions_dir = root.join("versions");
    fs::create_dir_all(&versions_dir).map_err(RuntimeError::internal)?;

    let stage_name = format!(".staging-{}-{}", asset.runtime_tag, std::process::id());
    let stage = root.join(stage_name);
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(RuntimeError::internal)?;
    }
    fs::create_dir_all(&stage).map_err(RuntimeError::internal)?;

    let install_result = install_staged_runtime(&asset, &stage, &versions_dir, &root).await;
    if install_result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    let server_relative_path = install_result?;
    let _ = fs::remove_dir_all(&stage);

    Ok(Json(DownloadResponse {
        runtime_tag: asset.runtime_tag,
        binary_verified: true,
        message: format!(
            "Runtime installed and verified at {}.",
            server_relative_path
        ),
    }))
}

async fn install_staged_runtime(
    asset: &RuntimeAsset,
    stage: &Path,
    versions_dir: &Path,
    runtime_root: &Path,
) -> Result<String, RuntimeError> {
    if asset.size == 0 || asset.size > 2 * 1024 * 1024 * 1024 {
        return Err(RuntimeError(
            StatusCode::BAD_GATEWAY,
            "The GitHub archive size is missing or exceeds the 2 GiB safety limit.".into(),
        ));
    }
    let archive_path = stage.join(&asset.name);
    download_and_verify(asset, &archive_path).await?;

    let extracted = stage.join("extracted");
    fs::create_dir_all(&extracted).map_err(RuntimeError::internal)?;
    extract_runtime_archive(&archive_path, &extracted).await?;

    let server_path = find_server_binary(&extracted)?;
    make_executable(&server_path)?;
    ensure_library_soname_links(&server_path).map_err(RuntimeError::internal)?;
    smoke_test_server(&server_path).await?;

    let generation = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let version_dir_name = format!("{}-{}", asset.runtime_tag, generation);
    let version_dir = versions_dir.join(&version_dir_name);
    fs::rename(&extracted, &version_dir).map_err(RuntimeError::internal)?;
    let server_relative_path = version_dir.join(
        server_path
            .strip_prefix(&extracted)
            .map_err(RuntimeError::internal)?,
    );
    let relative = server_relative_path
        .strip_prefix(runtime_root)
        .map_err(RuntimeError::internal)?
        .to_string_lossy()
        .to_string();

    let marker = serde_json::json!({
        "runtime_tag": asset.runtime_tag,
        "stable_version": asset.stable_version,
        "server_path": relative,
    });
    let marker_temp = runtime_root.join("runtime.json.tmp");
    let marker_path = runtime_root.join("runtime.json");
    fs::write(
        &marker_temp,
        serde_json::to_vec_pretty(&marker).map_err(RuntimeError::internal)?,
    )
    .map_err(RuntimeError::internal)?;
    fs::rename(&marker_temp, &marker_path).map_err(RuntimeError::internal)?;
    Ok(relative)
}

async fn download_and_verify(asset: &RuntimeAsset, destination: &Path) -> Result<(), RuntimeError> {
    let response = github_client()
        .get(&asset.url)
        .send()
        .await
        .map_err(RuntimeError::upstream)?
        .error_for_status()
        .map_err(RuntimeError::upstream)?;
    if let Some(length) = response.content_length() {
        if length != asset.size {
            return Err(RuntimeError(
                StatusCode::BAD_GATEWAY,
                "The archive size differs from GitHub release metadata.".into(),
            ));
        }
    }
    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(RuntimeError::internal)?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(RuntimeError::upstream)?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > asset.size || downloaded > 2 * 1024 * 1024 * 1024 {
            return Err(RuntimeError(
                StatusCode::BAD_GATEWAY,
                "The downloaded archive exceeded its declared size.".into(),
            ));
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(RuntimeError::internal)?;
    }
    file.flush().await.map_err(RuntimeError::internal)?;
    if downloaded != asset.size {
        return Err(RuntimeError(
            StatusCode::BAD_GATEWAY,
            "The download was incomplete. Retry the download.".into(),
        ));
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual != asset.sha256 {
        return Err(RuntimeError(
            StatusCode::BAD_GATEWAY,
            "The archive checksum did not match GitHub's SHA-256 digest.".into(),
        ));
    }
    Ok(())
}

async fn resolve_cpu_asset() -> Result<RuntimeAsset, RuntimeError> {
    if !is_supported_cpu_target() {
        return Err(RuntimeError(
            StatusCode::NOT_IMPLEMENTED,
            format!(
                "No built-in CPU runtime archive is configured for {}.",
                platform_label()
            ),
        ));
    }
    let (stable_version, stable_url, nightly_tag) = resolve_stable_release().await?;

    let nightly_url =
        format!("https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/{nightly_tag}");
    let nightly: serde_json::Value = github_client()
        .get(nightly_url)
        .send()
        .await
        .map_err(RuntimeError::upstream)?
        .error_for_status()
        .map_err(RuntimeError::upstream)?
        .json()
        .await
        .map_err(RuntimeError::upstream)?;
    let expected_asset = cpu_asset_name(&nightly_tag).ok_or_else(|| {
        RuntimeError(
            StatusCode::NOT_IMPLEMENTED,
            format!(
                "No built-in CPU runtime archive is configured for {}.",
                platform_label()
            ),
        )
    })?;
    let asset = nightly
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .and_then(|assets| {
            assets.iter().find(|item| {
                item.get("name").and_then(serde_json::Value::as_str) == Some(&expected_asset)
            })
        })
        .ok_or_else(|| {
            RuntimeError(
                StatusCode::BAD_GATEWAY,
                format!("The linked nightly release has no CPU archive named {expected_asset}."),
            )
        })?;
    let digest = asset
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| {
            RuntimeError(
                StatusCode::BAD_GATEWAY,
                "GitHub did not provide a SHA-256 digest for the CPU archive.".into(),
            )
        })?;

    Ok(RuntimeAsset {
        stable_version,
        stable_url,
        runtime_tag: nightly_tag,
        name: expected_asset,
        url: asset
            .get("browser_download_url")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                RuntimeError(
                    StatusCode::BAD_GATEWAY,
                    "The runtime asset did not include a download URL.".into(),
                )
            })?
            .to_owned(),
        size: asset
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                RuntimeError(
                    StatusCode::BAD_GATEWAY,
                    "The runtime asset did not include a file size.".into(),
                )
            })?,
        sha256: digest.to_ascii_lowercase(),
    })
}

async fn resolve_stable_release() -> Result<(String, String, String), RuntimeError> {
    let stable: serde_json::Value = github_client()
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")
        .send()
        .await
        .map_err(RuntimeError::upstream)?
        .error_for_status()
        .map_err(RuntimeError::upstream)?
        .json()
        .await
        .map_err(RuntimeError::upstream)?;
    let stable_tag = stable
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .filter(|tag| parse_stable_version(tag).is_some())
        .ok_or_else(|| {
            RuntimeError(
                StatusCode::BAD_GATEWAY,
                "GitHub's latest llama.cpp release did not have a stable vX.Y.Z tag.".into(),
            )
        })?;
    let stable_url = stable
        .get("html_url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            RuntimeError(
                StatusCode::BAD_GATEWAY,
                "The stable release did not include a release notes URL.".into(),
            )
        })?
        .to_owned();
    let body = stable
        .get("body")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let nightly_tag = extract_nightly_tag(body).ok_or_else(|| {
        RuntimeError(
            StatusCode::BAD_GATEWAY,
            "The stable release notes did not link to a nightly build tag.".into(),
        )
    })?;
    Ok((stable_tag.to_owned(), stable_url, nightly_tag))
}

fn is_supported_cpu_target() -> bool {
    (cfg!(target_os = "linux") || cfg!(target_os = "windows"))
        && matches!(std::env::consts::ARCH, "x86_64" | "aarch64")
}

fn cpu_asset_name(tag: &str) -> Option<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    #[cfg(target_os = "linux")]
    {
        Some(format!("llama-{tag}-bin-ubuntu-{arch}.tar.gz"))
    }
    #[cfg(target_os = "windows")]
    {
        Some(format!("llama-{tag}-bin-win-cpu-{arch}.zip"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (tag, arch);
        None
    }
}

async fn extract_runtime_archive(
    archive_path: &Path,
    destination: &Path,
) -> Result<(), RuntimeError> {
    #[cfg(target_os = "windows")]
    {
        let archive_path = archive_path.to_path_buf();
        let destination = destination.to_path_buf();
        tokio::task::spawn_blocking(move || {
            use std::process::Command;
            let status = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", "Expand-Archive -LiteralPath $env:STRAWBERRY_ARCHIVE -DestinationPath $env:STRAWBERRY_EXTRACT -Force; Get-ChildItem -LiteralPath $env:STRAWBERRY_EXTRACT -Recurse -File | Unblock-File"])
                .env("STRAWBERRY_ARCHIVE", archive_path)
                .env("STRAWBERRY_EXTRACT", destination)
                .status().map_err(RuntimeError::internal)?;
            if !status.success() { return Err(RuntimeError(StatusCode::BAD_GATEWAY, "Windows could not extract the verified runtime archive.".into())); }
            Ok::<(), RuntimeError>(())
        }).await.map_err(RuntimeError::internal)??;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let archive_path = archive_path.to_path_buf();
        let destination = destination.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let file = fs::File::open(archive_path).map_err(RuntimeError::internal)?;
            Archive::new(GzDecoder::new(file))
                .unpack(destination)
                .map_err(RuntimeError::internal)
        })
        .await
        .map_err(RuntimeError::internal)??;
    }
    Ok(())
}

fn extract_nightly_tag(body: &str) -> Option<String> {
    let line = body
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("nightly build"))?;
    let marker = "/releases/tag/";
    let start = line.find(marker)? + marker.len();
    let tag: String = line[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '.' || *ch == '-' || *ch == '_')
        .collect();
    let suffix = tag.strip_prefix('b')?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(tag)
}

fn parse_stable_version(tag: &str) -> Option<(u64, u64, u64)> {
    let version = tag.strip_prefix('v')?;
    let mut parts = version.split('.');
    let parse_part = |part: &str| {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let major = parse_part(parts.next()?)?;
    let minor = parse_part(parts.next()?)?;
    let patch = parse_part(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn find_server_binary(root: &Path) -> Result<PathBuf, RuntimeError> {
    fn walk(path: &Path) -> Option<PathBuf> {
        let entries = fs::read_dir(path).ok()?;
        for entry in entries.flatten() {
            let item = entry.path();
            if item.is_dir() {
                if let Some(found) = walk(&item) {
                    return Some(found);
                }
            } else if item.file_name().and_then(|name| name.to_str())
                == Some(if cfg!(target_os = "windows") {
                    "llama-server.exe"
                } else {
                    "llama-server"
                })
            {
                return Some(item);
            }
        }
        None
    }
    walk(root).ok_or_else(|| {
        RuntimeError(
            StatusCode::BAD_GATEWAY,
            "The archive did not contain a llama-server executable.".into(),
        )
    })
}

pub(crate) fn ensure_library_soname_links(server_path: &Path) -> std::io::Result<()> {
    #[cfg(not(unix))]
    let _ = server_path;
    #[cfg(unix)]
    {
        let Some(directory) = server_path.parent() else {
            return Ok(());
        };
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some((stem, version)) = name.split_once(".so.") else {
                continue;
            };
            let Some(major) = version.split('.').next().filter(|value| !value.is_empty()) else {
                continue;
            };
            if !major.chars().all(|value| value.is_ascii_digit()) {
                continue;
            }
            let soname = directory.join(format!("{stem}.so.{major}"));
            if !soname.exists() {
                std::os::unix::fs::symlink(entry.file_name(), soname)?;
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), RuntimeError> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(RuntimeError::internal)?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions).map_err(RuntimeError::internal)
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), RuntimeError> {
    Ok(())
}

async fn smoke_test_server(path: &Path) -> Result<(), RuntimeError> {
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--version")
        .current_dir(path.parent().unwrap_or(Path::new(".")));
    #[cfg(target_os = "linux")]
    if let Some(parent) = path.parent() {
        command.env("LD_LIBRARY_PATH", parent);
    }
    let output = command.output().await.map_err(|error| {
        RuntimeError(
            StatusCode::BAD_GATEWAY,
            format!("Could not start llama-server for its verification check: {error}"),
        )
    })?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr);
        return Err(RuntimeError(
            StatusCode::BAD_GATEWAY,
            format!(
                "The downloaded llama-server failed its version check: {}",
                details.trim()
            ),
        ));
    }
    Ok(())
}

fn read_installed_runtime_tag(data_dir: &Path) -> Option<String> {
    let marker = fs::read(data_dir.join("llama.cpp/runtime.json")).ok()?;
    serde_json::from_slice::<serde_json::Value>(&marker)
        .ok()?
        .get("runtime_tag")?
        .as_str()
        .map(str::to_owned)
}

fn read_auto_download_preference(data_dir: &Path) -> bool {
    crate::db::Database::init(data_dir.join("sessions/chats.db"))
        .and_then(|db| db.get_preference("auto_download_runtime"))
        .ok()
        .flatten()
        .as_deref()
        == Some("true")
}

fn github_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Strawberry-Social")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn platform_label() -> String {
    format!(
        "{} / {}",
        platform_name(std::env::consts::OS),
        architecture_name(std::env::consts::ARCH)
    )
}
fn platform_name(os: &str) -> &str {
    match os {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}
fn architecture_name(arch: &str) -> &str {
    match arch {
        "x86_64" => "x86_64",
        "aarch64" => "ARM64",
        other => other,
    }
}

struct RuntimeError(StatusCode, String);
impl RuntimeError {
    fn internal(error: impl std::fmt::Display) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
    fn upstream(error: impl std::fmt::Display) -> Self {
        Self(StatusCode::BAD_GATEWAY, error.to_string())
    }
}
impl IntoResponse for RuntimeError {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1).into_response()
    }
}
