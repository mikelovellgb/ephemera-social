//! JSON-RPC HTTP handler that proxies requests to the embedded node's router.
//!
//! Exposes a single POST `/rpc` endpoint. The frontend sends standard
//! JSON-RPC 2.0 requests, which are dispatched through the node's
//! [`Router`](ephemera_node::rpc::Router) and returned as JSON-RPC responses.
//!
//! Every request must include an `Authorization: Bearer <hex_token>` header
//! containing the RPC authentication token generated at node startup.

use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use ephemera_node::rpc::{JsonRpcRequest, JsonRpcResponse};
use serde_json::Value;
use std::sync::Arc;

/// Handle a JSON-RPC 2.0 request over HTTP POST.
///
/// Validates the `Authorization` header, parses the raw JSON body as a
/// `JsonRpcRequest`, dispatches it through the node's router, and returns
/// the `JsonRpcResponse`.
pub async fn handle_rpc(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    // Validate RPC authentication token.
    let authorized = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| state.rpc_auth.validate_bearer(v));

    if !authorized {
        let response = JsonRpcResponse::error(
            Value::Null,
            -32000,
            "Unauthorized: missing or invalid RPC authentication token",
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::to_value(response).unwrap_or_default()),
        );
    }

    // Parse the incoming JSON as a JsonRpcRequest
    let request: JsonRpcRequest = match serde_json::from_value(body) {
        Ok(req) => req,
        Err(e) => {
            let response = JsonRpcResponse::error(Value::Null, -32700, format!("Parse error: {e}"));
            return (
                StatusCode::OK,
                Json(serde_json::to_value(response).unwrap_or_default()),
            );
        }
    };

    tracing::debug!(method = %request.method, "RPC request");

    // Dispatch through the node's JSON-RPC router
    let response: JsonRpcResponse = state.router.dispatch(request).await;

    let json_value = serde_json::to_value(&response).unwrap_or_else(|e| {
        tracing::error!(error = %e, "failed to serialize RPC response");
        serde_json::json!({
            "jsonrpc": "2.0",
            "error": { "code": -32603, "message": "internal serialization error" },
            "id": null
        })
    });

    (StatusCode::OK, Json(json_value))
}
