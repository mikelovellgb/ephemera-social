//! JSON-RPC handlers for the `network.*` namespace.
//!
//! Provides peer connection management, network info, and transport details.
//!
//! The `network.connect` method accepts either:
//! - `addr`: an IP:port string (TCP or Iroh with direct addresses)
//! - `node_id`: a hex-encoded 32-byte Ed25519 public key (Iroh discovery)
//!
//! When using Iroh transport, connecting by `node_id` alone is preferred
//! because Iroh will resolve the peer's address via relay/discovery.

use crate::network::{NetworkSubsystem, TransportKind};
use crate::rpc::{error_codes, JsonRpcError, Router};
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

/// Try to extract an optional string parameter from the JSON-RPC params.
fn extract_str_opt(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Register `network.*` namespace methods.
///
/// These methods require a running [`NetworkSubsystem`]. If the network is
/// not available (node not started), callers receive a `NETWORK_UNAVAILABLE`
/// error.
pub fn register_network(router: &mut Router, network: &Arc<NetworkSubsystem>) {
    // network.info() -- report transport type, local NodeId, peer count
    let net = Arc::clone(network);
    router.register("network.info", move |_params| {
        let net = Arc::clone(&net);
        async move {
            let kind = net.transport_kind();
            let transport_name = match kind {
                TransportKind::Tcp => "tcp",
                #[cfg(feature = "iroh-transport")]
                TransportKind::Iroh => "iroh",
            };
            Ok(serde_json::json!({
                "transport": transport_name,
                "node_id": net.local_id().to_string(),
                "peer_count": net.peer_count(),
            }))
        }
    });

    // network.connect -- connect to a remote peer
    //
    // Accepts either:
    //   { "addr": "1.2.3.4:9100" }           -- TCP-style, works with any backend
    //   { "node_id": "<64-char-hex>" }        -- Iroh discovery (no IP needed)
    //   { "node_id": "...", "addr": "..." }   -- Iroh with direct address hint
    let net = Arc::clone(network);
    router.register("network.connect", move |params| {
        let net = Arc::clone(&net);
        async move {
            let addr_opt = extract_str_opt(&params, "addr");
            let node_id_hex_opt = extract_str_opt(&params, "node_id");

            let peer_addr = match (node_id_hex_opt, addr_opt) {
                // Iroh-style: connect by NodeId (with optional address hint)
                (Some(node_id_hex), addr) => {
                    let peer_bytes = hex::decode(&node_id_hex).map_err(|e| JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: format!("invalid node_id hex: {e}"),
                        data: None,
                    })?;
                    if peer_bytes.len() != 32 {
                        return Err(JsonRpcError {
                            code: error_codes::INVALID_PARAMS,
                            message: format!(
                                "node_id must be 32 bytes (64 hex chars), got {} bytes",
                                peer_bytes.len()
                            ),
                            data: None,
                        });
                    }
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&peer_bytes);
                    ephemera_transport::PeerAddr {
                        node_id: ephemera_types::NodeId::from_bytes(arr),
                        addresses: addr.into_iter().collect(),
                    }
                }
                // TCP-style: connect by address only (node_id placeholder)
                (None, Some(addr)) => ephemera_transport::PeerAddr {
                    node_id: ephemera_types::NodeId::from_bytes([0u8; 32]),
                    addresses: vec![addr],
                },
                // Neither provided
                (None, None) => {
                    return Err(JsonRpcError {
                        code: error_codes::INVALID_PARAMS,
                        message: "must provide 'addr' and/or 'node_id'".into(),
                        data: None,
                    });
                }
            };

            net.connect_to_peer(&peer_addr)
                .await
                .map_err(|e| JsonRpcError {
                    code: error_codes::NETWORK_UNAVAILABLE,
                    message: format!("connect failed: {e}"),
                    data: None,
                })?;

            // After connection, list peers to find the newly connected one.
            let peers = net.connected_peers();
            let peer_id = peers
                .last()
                .map(|id| id.to_string())
                .unwrap_or_default();
            Ok(serde_json::json!({
                "ok": true,
                "peer_id": peer_id,
            }))
        }
    });

    // network.peers() -- list connected peers with addresses
    let net = Arc::clone(network);
    router.register("network.peers", move |_params| {
        let net = Arc::clone(&net);
        async move {
            let peers = net.connected_peers();
            let peer_list: Vec<Value> = peers
                .iter()
                .map(|id| {
                    serde_json::json!({
                        "peer_id": id.to_string(),
                    })
                })
                .collect();
            Ok(serde_json::json!({
                "peers": peer_list,
                "count": peer_list.len(),
            }))
        }
    });

    // network.disconnect(peer_id) -- disconnect a specific peer
    let net = Arc::clone(network);
    router.register("network.disconnect", move |params| {
        let net = Arc::clone(&net);
        async move {
            let peer_id_hex = extract_str(&params, "peer_id")?;
            let peer_bytes = hex::decode(&peer_id_hex).map_err(|e| JsonRpcError {
                code: error_codes::INVALID_PARAMS,
                message: format!("invalid peer_id hex: {e}"),
                data: None,
            })?;
            if peer_bytes.len() != 32 {
                return Err(JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: format!(
                        "peer_id must be 32 bytes (64 hex chars), got {} bytes",
                        peer_bytes.len()
                    ),
                    data: None,
                });
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&peer_bytes);
            let node_id = ephemera_types::NodeId::from_bytes(arr);
            net.disconnect_peer(&node_id).await.map_err(|e| JsonRpcError {
                code: error_codes::NETWORK_UNAVAILABLE,
                message: format!("disconnect failed: {e}"),
                data: None,
            })?;
            Ok(serde_json::json!({ "ok": true, "peer_id": peer_id_hex }))
        }
    });

    // network.status() -- comprehensive diagnostic: transport, node_id,
    // peer_count, iroh availability, and any error.
    let net = Arc::clone(network);
    router.register("network.status", move |_params| {
        let net = Arc::clone(&net);
        async move {
            let kind = net.transport_kind();
            let transport_name = match kind {
                TransportKind::Tcp => "tcp",
                #[cfg(feature = "iroh-transport")]
                TransportKind::Iroh => "iroh",
            };
            let iroh_available = match kind {
                #[cfg(feature = "iroh-transport")]
                TransportKind::Iroh => true,
                _ => false,
            };
            Ok(serde_json::json!({
                "transport": transport_name,
                "node_id": net.local_id().to_string(),
                "peer_count": net.peer_count(),
                "iroh_available": iroh_available,
                "error": Value::Null,
            }))
        }
    });
}

/// Register stub `network.*` methods that return a descriptive error.
///
/// Called when the network subsystem is not available (e.g. identity locked
/// at startup, transport failed to initialize). This ensures the methods
/// exist so clients get a proper JSON-RPC error instead of "method not found".
pub fn register_network_stubs(router: &mut Router) {
    let unavailable = || JsonRpcError {
        code: error_codes::NETWORK_UNAVAILABLE,
        message: "network subsystem not available — unlock identity and restart node".into(),
        data: None,
    };

    router.register("network.info", move |_params| {
        async move { Err::<Value, _>(unavailable()) }
    });

    router.register("network.connect", move |_params| {
        async move { Err::<Value, _>(unavailable()) }
    });

    router.register("network.peers", move |_params| {
        async move { Err::<Value, _>(unavailable()) }
    });

    router.register("network.disconnect", move |_params| {
        async move { Err::<Value, _>(unavailable()) }
    });

    // network.status returns a valid response even when the network is down,
    // so the frontend can always show diagnostic info.
    router.register("network.status", move |_params| {
        async move {
            Ok(serde_json::json!({
                "transport": "none",
                "node_id": Value::Null,
                "peer_count": 0,
                "iroh_available": false,
                "error": "network subsystem not available — unlock identity and restart node",
            }))
        }
    });
}
