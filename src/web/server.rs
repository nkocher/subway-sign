use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use rust_embed::Embed;
use tracing::info;

use crate::AppState;

use super::handlers;

/// Embedded web assets (HTML, CSS, JS, icons).
#[derive(Embed)]
#[folder = "web/"]
#[prefix = ""]
struct WebAssets;

/// Run the axum web server on 0.0.0.0:5001.
pub async fn run(state: Arc<AppState>) {
    let app = Router::new()
        // API routes
        .route("/api/config", get(handlers::get_config).post(handlers::update_config))
        .route("/api/status", get(handlers::get_status))
        .route("/api/stations/complete", get(handlers::get_complete_stations))
        .route("/api/stations/lookup/{station_name}", get(handlers::lookup_station))
        .route("/api/stations", get(handlers::get_stations))
        .route("/api/restart", post(handlers::restart))
        // Static files and index
        .route("/", get(serve_index))
        .fallback(get(serve_static))
        // Middleware
        .layer(DefaultBodyLimit::max(65536)) // 64KB max request body
        // Shared state
        .with_state(state.clone());

    let listener = match tokio::net::TcpListener::bind("0.0.0.0:5001").await {
        Ok(l) => {
            info!("[WEB] Server listening on http://0.0.0.0:5001");
            l
        }
        Err(e) => {
            tracing::error!("[WEB] Failed to bind port 5001: {}", e);
            return;
        }
    };

    let shutdown = state.shutdown.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .ok();

    info!("[WEB] Server stopped");
}

/// Serve the main index.html page.
async fn serve_index() -> Response {
    serve_embedded_file("templates/index.html").await
}

/// Serve static files from embedded assets.
async fn serve_static(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    serve_embedded_file(path).await
}

/// Look up and serve an embedded file with appropriate content type.
async fn serve_embedded_file(path: &str) -> Response {
    match WebAssets::get(path) {
        Some(file) => {
            let mime = mime_for_path(path);
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime)],
                file.data.to_vec(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Determine MIME type from file extension.
fn mime_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("svg") => "image/svg+xml",
        Some("webmanifest") | Some("manifest") => "application/manifest+json",
        _ => "application/octet-stream",
    }
}
