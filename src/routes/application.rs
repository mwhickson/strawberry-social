use crate::AppState;
use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc, time::Duration};

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/shutdown", post(shutdown))
}

#[derive(Serialize)]
struct ShutdownResponse {
    detail: &'static str,
}

async fn shutdown(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<(StatusCode, Json<ShutdownResponse>), StatusCode> {
    if !peer.ip().is_loopback() {
        return Err(StatusCode::FORBIDDEN);
    }

    let sender = state
        .shutdown
        .lock()
        .await
        .take()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Let the HTTP response reach the UI before graceful server shutdown begins.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = sender.send(());
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(ShutdownResponse {
            detail: "Strawberry Social is closing.",
        }),
    ))
}
