//! JSON-RPC handlers for the `network.*` namespace.
//!
//! Provides peer connection management, network info, and transport details.
//!
//! The `network.connect` method accepts either:
//! - `addr`: an IP:port string (with direct addresses)
//! - `node_id`: a hex-encoded 32-byte Ed25519 public key (Iroh discovery)

use crate::network::{NetworkSubsystem, RelayState};
use crate::rpc::{error_codes, JsonRpcError, Router};
use crate::services::ServiceContainer;
use ephemera_transport::PeerAddr;
use ephemera_types::NodeId;
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

/// Register network methods that dynamically read from `services.network`.
/// When network is `None` (identity locked), returns appropriate offline status.
pub fn register_network_dynamic(router: &mut Router, services: &Arc<ServiceContainer>) {
    // Helper: get the current network or return an error.
    fn get_net(svc: &ServiceContainer) -> Result<Arc<NetworkSubsystem>, JsonRpcError> {
        svc.network
            .lock()
            .map_err(|_| JsonRpcError {
                code: error_codes::INTERNAL_ERROR,
                message: "network lock poisoned".into(),
                data: None,
            })?
            .clone()
            .ok_or_else(|| JsonRpcError {
                code: error_codes::NETWORK_UNAVAILABLE,
                message: "network not available — unlock identity first".into(),
                data: None,
            })
    }

    // network.info
    let svc = Arc::clone(services);
    router.register("network.info", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let net = get_net(&svc)?;
            Ok(serde_json::json!({
                "transport": "iroh",
                "node_id": net.local_id().to_string(),
                "peer_count": net.peer_count(),
            }))
        }
    });

    // network.status
    let svc = Arc::clone(services);
    router.register("network.status", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let guard = svc.network.lock().map_err(|_| JsonRpcError {
                code: error_codes::INTERNAL_ERROR,
                message: "lock".into(),
                data: None,
            })?;
            match &*guard {
                Some(net) => {
                    let relay_status = match net.relay_state() {
                        RelayState::Connected => "connected",
                        RelayState::Unavailable => "unavailable",
                    };
                    Ok(serde_json::json!({
                        "transport": "iroh",
                        "node_id": net.local_id().to_string(),
                        "peer_count": net.peer_count(),
                        "iroh_available": true,
                        "relay_status": relay_status,
                        "error": Value::Null,
                    }))
                }
                None => Ok(serde_json::json!({
                    "transport": "none",
                    "node_id": Value::Null,
                    "peer_count": 0,
                    "iroh_available": false,
                    "relay_status": "not_applicable",
                    "error": "network not started — unlock identity",
                })),
            }
        }
    });

    // network.connect
    let svc = Arc::clone(services);
    router.register("network.connect", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let net = get_net(&svc)?;

            let addr = params.get("addr").and_then(|v| v.as_str()).map(String::from);
            let node_id = params.get("node_id").and_then(|v| v.as_str()).map(String::from);
            if let Some(nid) = node_id {
                let nid_bytes = hex::decode(&nid).map_err(|e| JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: format!("bad node_id hex: {e}"),
                    data: None,
                })?;
                if nid_bytes.len() != 32 {
                    return Err(JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: format!("node_id must be 32 bytes, got {}", nid_bytes.len()),
                        data: None,
                    });
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&nid_bytes);
                let peer_addr = PeerAddr {
                    node_id: NodeId::from_bytes(arr),
                    addresses: addr.into_iter().collect(),
                };
                net.connect_to_peer(&peer_addr).await.map_err(|e| JsonRpcError {
                    code: error_codes::INTERNAL_ERROR,
                    message: format!("connect failed: {e}"),
                    data: None,
                })?;
                Ok(serde_json::json!({"connected": true, "node_id": nid}))
            } else if let Some(a) = addr {
                let peer_addr = PeerAddr {
                    node_id: NodeId::from_bytes([0u8; 32]),
                    addresses: vec![a.clone()],
                };
                net.connect_to_peer(&peer_addr).await.map_err(|e| JsonRpcError {
                    code: error_codes::INTERNAL_ERROR,
                    message: format!("connect failed: {e}"),
                    data: None,
                })?;
                Ok(serde_json::json!({"connected": true, "addr": a}))
            } else {
                Err(JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: "provide addr or node_id".into(),
                    data: None,
                })
            }
        }
    });

    // network.peers
    let svc = Arc::clone(services);
    router.register("network.peers", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let net = get_net(&svc)?;
            let peers = net.connected_peers();
            let list: Vec<Value> = peers.iter().map(|p| {
                serde_json::json!({"node_id": hex::encode(p.as_bytes())})
            }).collect();
            Ok(serde_json::json!({"peers": list, "count": list.len()}))
        }
    });

    // network.disconnect
    let svc = Arc::clone(services);
    router.register("network.disconnect", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let net = get_net(&svc)?;
            let peer_id_hex = extract_str(&params, "peer_id")?;
            let peer_bytes = hex::decode(&peer_id_hex).map_err(|e| JsonRpcError {
                code: error_codes::INVALID_PARAMS,
                message: format!("invalid peer_id hex: {e}"),
                data: None,
            })?;
            if peer_bytes.len() != 32 {
                return Err(JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: format!("peer_id must be 32 bytes, got {}", peer_bytes.len()),
                    data: None,
                });
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&peer_bytes);
            let node_id = NodeId::from_bytes(arr);
            net.disconnect_peer(&node_id).await.map_err(|e| JsonRpcError {
                code: error_codes::NETWORK_UNAVAILABLE,
                message: format!("disconnect failed: {e}"),
                data: None,
            })?;
            Ok(serde_json::json!({ "ok": true, "peer_id": peer_id_hex }))
        }
    });
}
