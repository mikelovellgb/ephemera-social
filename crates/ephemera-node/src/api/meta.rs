//! JSON-RPC handlers for the `meta.*` namespace.
//!
//! Provides node status, capabilities listing, and transport tier switching.

use crate::rpc::{error_codes, JsonRpcError, Router};
use crate::services::ServiceContainer;
use serde_json::Value;
use std::sync::Arc;

/// Extract a required string parameter from the JSON-RPC params.
fn extract_str(params: &Value, key: &str) -> Result<String, JsonRpcError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| JsonRpcError {
            code: error_codes::INVALID_PARAMS,
            message: format!("missing or invalid parameter: {key}"),
            data: None,
        })
}

/// Register `meta.*` namespace methods.
///
/// `method_names` is a pre-collected list of all registered method names
/// (for the `meta.capabilities` response).
pub fn register_meta(
    router: &mut Router,
    services: &Arc<ServiceContainer>,
    method_names: Vec<String>,
) {
    let caps = method_names;
    router.register("meta.capabilities", move |_params| {
        let caps = caps.clone();
        async move {
            Ok(serde_json::json!({
                "protocol_version": 1,
                "api_version": "0.1.0",
                "methods": caps,
                "capabilities": [
                    "identity", "posts", "feed", "social",
                    "messages", "profiles", "moderation", "meta",
                    "discover"
                ],
            }))
        }
    });

    let svc = Arc::clone(services);
    router.register("meta.status", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            Ok(serde_json::json!({
                "node_id": "not-yet-assigned",
                "uptime_seconds": svc.uptime_secs(),
                "peer_count": 0,
                "connected_relays": 0,
                "transport_tier": "T2",
                "storage_used_bytes": 0,
                "storage_cap_bytes": 524_288_000_u64,
                "network_status": "disconnected",
                "sync_status": "idle",
            }))
        }
    });

    router.register("meta.set_transport_tier", |params| async move {
        let _tier = extract_str(&params, "tier")?;
        // TODO: actually switch transport tier via TransportManager
        Ok(serde_json::json!({ "ok": true }))
    });
}
