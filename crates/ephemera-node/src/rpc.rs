//! JSON-RPC 2.0 request/response types and method router.
//!
//! Implements the standard JSON-RPC 2.0 protocol for the Ephemera API.
//! The router dispatches method names (e.g. `"posts.create"`) to the
//! appropriate handler function in [`crate::api`].

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Standard JSON-RPC 2.0 error codes.
pub mod error_codes {
    /// Invalid JSON was received by the server.
    pub const PARSE_ERROR: i64 = -32700;
    /// The JSON sent is not a valid Request object.
    pub const INVALID_REQUEST: i64 = -32600;
    /// The method does not exist / is not available.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// Invalid method parameter(s).
    pub const INVALID_PARAMS: i64 = -32602;
    /// Internal JSON-RPC error.
    pub const INTERNAL_ERROR: i64 = -32603;

    // Application-specific error codes
    /// Authentication required (keystore locked).
    pub const AUTH_REQUIRED: i64 = -1001;
    /// Rate limited.
    pub const RATE_LIMITED: i64 = -1002;
    /// Content validation failed.
    pub const CONTENT_VALIDATION_FAILED: i64 = -1003;
    /// Network unavailable.
    pub const NETWORK_UNAVAILABLE: i64 = -1004;
    /// Content not found.
    pub const CONTENT_NOT_FOUND: i64 = -1005;
    /// Permission denied.
    pub const PERMISSION_DENIED: i64 = -1006;
    /// PoW computation failed.
    pub const POW_FAILED: i64 = -1007;
}

/// A JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Protocol version, always "2.0".
    pub jsonrpc: String,
    /// Method name in "namespace.method" format.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Value,
    /// Request identifier (number or string).
    pub id: Value,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// Protocol version, always "2.0".
    pub jsonrpc: String,
    /// Result value (present on success).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error object (present on failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    /// Request identifier, echoed back.
    pub id: Value,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Numeric error code.
    pub code: i64,
    /// Human-readable error message.
    pub message: String,
    /// Optional additional data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    /// Create a success response.
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Create an error response.
    pub fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }

    /// Create an error response with additional data.
    pub fn error_with_data(id: Value, code: i64, message: impl Into<String>, data: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: Some(data),
            }),
            id,
        }
    }
}

/// Type alias for async handler functions.
type HandlerFn = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, JsonRpcError>> + Send>>
        + Send
        + Sync,
>;

/// JSON-RPC method router.
///
/// Maps method names like `"identity.create"` to async handler functions.
/// Cloneable because handlers are behind `Arc`.
#[derive(Clone)]
pub struct Router {
    handlers: HashMap<String, HandlerFn>,
}

impl Router {
    /// Create an empty router.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for a method name.
    pub fn register<F, Fut>(&mut self, method: &str, handler: F)
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, JsonRpcError>> + Send + 'static,
    {
        let handler = Arc::new(move |params: Value| {
            let fut = handler(params);
            Box::pin(fut) as Pin<Box<dyn Future<Output = Result<Value, JsonRpcError>> + Send>>
        });
        self.handlers.insert(method.to_string(), handler);
    }

    /// Dispatch a JSON-RPC request to the appropriate handler.
    pub async fn dispatch(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        if request.jsonrpc != "2.0" {
            return JsonRpcResponse::error(
                request.id,
                error_codes::INVALID_REQUEST,
                "unsupported JSON-RPC version",
            );
        }

        let handler = match self.handlers.get(&request.method) {
            Some(h) => h.clone(),
            None => {
                return JsonRpcResponse::error(
                    request.id,
                    error_codes::METHOD_NOT_FOUND,
                    format!("method not found: {}", request.method),
                );
            }
        };

        match handler(request.params).await {
            Ok(result) => JsonRpcResponse::success(request.id, result),
            Err(rpc_err) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(rpc_err),
                id: request.id,
            },
        }
    }

    /// List all registered method names.
    pub fn method_names(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_response_serializes() {
        let resp =
            JsonRpcResponse::success(Value::Number(1.into()), serde_json::json!({"ok": true}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn error_response_serializes() {
        let resp = JsonRpcResponse::error(
            Value::Number(1.into()),
            error_codes::METHOD_NOT_FOUND,
            "not found",
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\""));
        assert!(!json.contains("\"result\""));
    }

    #[tokio::test]
    async fn router_dispatches_to_handler() {
        let mut router = Router::new();
        router.register("test.echo", |params| async move { Ok(params) });

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "test.echo".to_string(),
            params: serde_json::json!({"message": "hello"}),
            id: Value::Number(1.into()),
        };

        let response = router.dispatch(request).await;
        assert!(response.error.is_none());
        assert_eq!(response.result.unwrap()["message"], "hello");
    }

    #[tokio::test]
    async fn router_returns_method_not_found() {
        let router = Router::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "nonexistent.method".to_string(),
            params: Value::Null,
            id: Value::Number(1.into()),
        };

        let response = router.dispatch(request).await;
        assert_eq!(response.error.unwrap().code, error_codes::METHOD_NOT_FOUND);
    }
}
