//! HTTP server that serves the embedded frontend and JSON-RPC endpoint.
//!
//! Uses axum to bind to a localhost port and serve:
//! - `POST /rpc` — JSON-RPC 2.0 endpoint proxying to the node
//! - `GET /media/:media_id` — Serves reassembled media blobs from chunk storage
//! - `GET /media/:media_id/thumbnail` — Serves media thumbnails
//! - `GET /*` — Static frontend assets (embedded via `rust-embed`)

use crate::commands::handle_rpc;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use rust_embed::Embed;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

/// Frontend assets embedded at compile time from the `frontend/` directory.
#[derive(Embed)]
#[folder = "frontend/"]
struct FrontendAssets;

/// Build the axum router with all routes.
///
/// Routes:
/// - `POST /rpc` — JSON-RPC handler
/// - `GET /*` — Static file server (falls back to index.html for SPA routing)
pub fn build_app(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/rpc", post(handle_rpc))
        .route("/media/{media_id}", get(serve_media))
        .route("/media/{media_id}/thumbnail", get(serve_thumbnail))
        .route("/{*path}", get(serve_static))
        .route("/", get(serve_index))
        .layer(cors)
        .with_state(state)
}

/// Serve the index.html file (SPA entry point).
///
/// Injects the RPC authentication token into the HTML so the frontend
/// JavaScript can authenticate its JSON-RPC requests. The token is set
/// as `window.__EPHEMERA_RPC_TOKEN__` via a `<script>` tag injected
/// just before `</head>`.
async fn serve_index(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let token_hex = state.rpc_auth.token_hex();
    inject_token_into_index(&token_hex)
}

/// Serve a static file from the embedded frontend assets.
///
/// Falls back to the token-injected `index.html` for SPA hash-routing support.
async fn serve_static(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    // Try the exact path first
    if let Some(resp) = try_serve_file(&path) {
        return resp;
    }

    // Fallback to index.html for SPA routing (with token injection)
    let token_hex = state.rpc_auth.token_hex();
    inject_token_into_index(&token_hex)
}

/// Read the embedded `index.html` and inject the RPC token as a script tag.
fn inject_token_into_index(token_hex: &str) -> Response<Body> {
    match FrontendAssets::get("index.html") {
        Some(file) => {
            let html = String::from_utf8_lossy(&file.data);
            let token_script = format!(
                "<script>window.__EPHEMERA_RPC_TOKEN__=\"{}\";</script>",
                token_hex
            );
            // Inject just before </head> so it's available before app.js runs
            let injected = html.replace("</head>", &format!("{token_script}\n</head>"));

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Body::from(injected))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("internal error"))
                        .expect("building error response should not fail")
                })
        }
        None => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from("index.html not found in embedded assets"))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("internal error"))
                    .expect("building error response should not fail")
            }),
    }
}

/// Serve a media attachment's reassembled bytes by media_id.
///
/// Reads all chunks for the given media attachment from the metadata DB,
/// reassembles them in order, and returns the complete media blob with the
/// correct MIME type.
async fn serve_media(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(media_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let services = state.node.services();
    let db = match services.metadata_db.lock() {
        Ok(d) => d,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("database lock error"))
                .expect("error response");
        }
    };

    // Look up the media attachment metadata.
    let attachment = match ephemera_store::get_media_attachment(&db, &media_id) {
        Ok(a) => a,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("media not found"))
                .expect("error response");
        }
    };

    // Retrieve and reassemble all chunks in order.
    let chunks = match ephemera_store::list_chunks_for_media(&db, &media_id) {
        Ok(c) => c,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("failed to read media chunks"))
                .expect("error response");
        }
    };

    let mut data = Vec::new();
    for chunk in &chunks {
        data.extend_from_slice(&chunk.data);
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &attachment.mime_type)
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .header(header::CONTENT_LENGTH, data.len().to_string())
        .body(Body::from(data))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("internal error"))
                .expect("error response")
        })
}

/// Serve a media attachment's thumbnail by media_id.
///
/// If the attachment has a thumbnail_hash, reads the thumbnail blob from the
/// content store and returns it as a PNG image.
async fn serve_thumbnail(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(media_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let services = state.node.services();
    let db = match services.metadata_db.lock() {
        Ok(d) => d,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("database lock error"))
                .expect("error response");
        }
    };

    // Look up the thumbnail hash.
    let thumb_hash = match ephemera_store::get_thumbnail_hash(&db, &media_id) {
        Ok(Some(h)) => h,
        Ok(None) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("no thumbnail"))
                .expect("error response");
        }
        Err(_) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("media not found"))
                .expect("error response");
        }
    };

    // Read the thumbnail blob from the content store.
    let content_store = services.content_store();
    match content_store.get(&thumb_hash) {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "image/png")
            .header(header::CACHE_CONTROL, "public, max-age=3600")
            .header(header::CONTENT_LENGTH, data.len().to_string())
            .body(Body::from(data))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("internal error"))
                    .expect("error response")
            }),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("thumbnail blob not found"))
            .expect("error response"),
    }
}

/// Attempt to serve an embedded file, returning None if not found.
fn try_serve_file(path: &str) -> Option<Response<Body>> {
    let file = FrontendAssets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();

    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .header(header::CACHE_CONTROL, "no-cache")
            .body(Body::from(file.data.to_vec()))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::from("internal error"))
                    .expect("building error response should not fail")
            }),
    )
}

/// Start the HTTP server on the given address.
///
/// # Errors
///
/// Returns an error if the server cannot bind to the address.
pub async fn start_server(
    state: Arc<AppState>,
    addr: SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!(%addr, "ephemera server listening");
    axum::serve(listener, app).await?;

    Ok(())
}
