//! JSON-RPC handlers for the `meta.*` namespace.
//!
//! Provides node status, capabilities listing, transport tier switching,
//! and the debug log endpoint for the in-app debug console.
//!
//! **Important**: All network reads go through `services.network` (the Mutex),
//! never a captured `Arc<NetworkSubsystem>`. This ensures handlers always see
//! the *current* network subsystem, even after `upgrade_network_to_iroh()`
//! replaces the Arc inside the Mutex.

use crate::debug_log::DebugLogHandle;
use crate::network::TransportKind;
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
///
/// Network status is always read from `services.network` (the Mutex) so that
/// handlers see the latest subsystem after a TCP-to-Iroh upgrade.
/// When `debug_log` is `Some`, the `meta.debug_log` endpoint serves
/// captured log entries from the ring buffer.
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
            let (node_id, peer_count, transport_tier, network_status) =
                match svc.network.lock() {
                    Ok(guard) => match &*guard {
                        Some(net) => {
                            let kind = net.transport_kind();
                            let tier = match kind {
                                TransportKind::Tcp => "T2",
                                #[cfg(feature = "iroh-transport")]
                                TransportKind::Iroh => "T1",
                            };
                            (
                                net.local_id().to_string(),
                                net.peer_count(),
                                tier,
                                "connected",
                            )
                        }
                        None => (
                            "not-yet-assigned".to_string(),
                            0,
                            "T2",
                            "disconnected",
                        ),
                    },
                    Err(_) => (
                        "not-yet-assigned".to_string(),
                        0,
                        "T2",
                        "disconnected",
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
                "sync_status": "idle",
            }))
        }
    });

    router.register("meta.set_transport_tier", |params| async move {
        let _tier = extract_str(&params, "tier")?;
        // TODO: actually switch transport tier via TransportManager
        Ok(serde_json::json!({ "ok": true }))
    });

    // meta.debug_log — returns the last N log entries and network status.
    //
    // Parameters:
    //   - count (optional, u64): number of log entries to return (default 50)
    //
    // Response:
    //   { logs: [...], network_status: { transport, node_id, relay, peer_count, iroh_active } }
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

            // Build network status from services.network (the Mutex), so we
            // always see the current subsystem even after a TCP → Iroh upgrade.
            let network_status = match svc.network.lock() {
                Ok(guard) => match &*guard {
                    Some(n) => {
                        let kind = n.transport_kind();
                        let transport_name = match kind {
                            TransportKind::Tcp => "tcp",
                            #[cfg(feature = "iroh-transport")]
                            TransportKind::Iroh => "iroh",
                        };
                        let iroh_active = match kind {
                            #[cfg(feature = "iroh-transport")]
                            TransportKind::Iroh => true,
                            _ => false,
                        };
                        serde_json::json!({
                            "transport": transport_name,
                            "node_id": n.local_id().to_string(),
                            "relay": Value::Null,
                            "peer_count": n.peer_count(),
                            "iroh_active": iroh_active,
                        })
                    }
                    None => {
                        serde_json::json!({
                            "transport": "none",
                            "node_id": Value::Null,
                            "relay": Value::Null,
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
