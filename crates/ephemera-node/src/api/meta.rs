//! JSON-RPC handlers for the `meta.*` namespace.
//!
//! Provides node status, capabilities listing, transport tier info,
//! and the debug log endpoint for the in-app debug console.
//!
//! All network reads go through `services.network` (the Mutex) so handlers
//! always see the current network subsystem.

use crate::debug_log::DebugLogHandle;
use crate::network::RelayState;
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
    debug_log: Option<DebugLogHandle>,
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

    // meta.status — reads live network state from services.network (Mutex).
    let svc = Arc::clone(services);
    router.register("meta.status", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let (node_id, peer_count, transport_tier, network_status, relay_status) =
                match svc.network.lock() {
                    Ok(guard) => match &*guard {
                        Some(net) => {
                            let relay = match net.relay_state() {
                                RelayState::Connected => "connected",
                                RelayState::Unavailable => "unavailable",
                            };
                            (
                                net.local_id().to_string(),
                                net.peer_count(),
                                "T1",
                                "connected",
                                relay,
                            )
                        }
                        None => (
                            "not-yet-assigned".to_string(),
                            0,
                            "offline",
                            "waiting_for_unlock",
                            "not_applicable",
                        ),
                    },
                    Err(_) => (
                        "not-yet-assigned".to_string(),
                        0,
                        "offline",
                        "waiting_for_unlock",
                        "not_applicable",
                    ),
                };

            Ok(serde_json::json!({
                "node_id": node_id,
                "uptime_seconds": svc.uptime_secs(),
                "peer_count": peer_count,
                "connected_relays": 0,
                "transport_tier": transport_tier,
                "storage_used_bytes": 0,
                "storage_cap_bytes": 524_288_000_u64,
                "network_status": network_status,
                "relay_status": relay_status,
                "sync_status": "idle",
            }))
        }
    });

    router.register("meta.set_transport_tier", |params| async move {
        let _tier = extract_str(&params, "tier")?;
        Ok(serde_json::json!({ "ok": true }))
    });

    // meta.debug_log — returns the last N log entries and network status.
    let svc_for_debug = Arc::clone(services);
    let log_handle = debug_log;
    router.register("meta.debug_log", move |params| {
        let svc = Arc::clone(&svc_for_debug);
        let handle = log_handle.clone();
        async move {
            let count = params
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize;

            // Collect log entries from the ring buffer.
            let logs: Vec<Value> = match &handle {
                Some(h) => h
                    .read_last(count)
                    .into_iter()
                    .map(|entry| {
                        serde_json::json!({
                            "level": entry.level,
                            "target": entry.target,
                            "message": entry.message,
                            "timestamp": entry.timestamp,
                        })
                    })
                    .collect(),
                None => vec![],
            };

            let network_status = match svc.network.lock() {
                Ok(guard) => match &*guard {
                    Some(n) => {
                        let relay_status = match n.relay_state() {
                            RelayState::Connected => "connected",
                            RelayState::Unavailable => "unavailable",
                        };
                        serde_json::json!({
                            "transport": "iroh",
                            "node_id": n.local_id().to_string(),
                            "relay": Value::Null,
                            "relay_status": relay_status,
                            "peer_count": n.peer_count(),
                            "iroh_active": true,
                        })
                    }
                    None => {
                        serde_json::json!({
                            "transport": "none",
                            "node_id": Value::Null,
                            "relay": Value::Null,
                            "relay_status": "not_applicable",
                            "peer_count": 0,
                            "iroh_active": false,
                        })
                    }
                },
                Err(_) => {
                    serde_json::json!({
                        "transport": "none",
                        "node_id": Value::Null,
                        "relay": Value::Null,
                        "relay_status": "not_applicable",
                        "peer_count": 0,
                        "iroh_active": false,
                    })
                }
            };

            Ok(serde_json::json!({
                "logs": logs,
                "network_status": network_status,
            }))
        }
    });
}
