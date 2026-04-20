//! Network DHT query/response protocol.
//!
//! Turns the local-only DHT into a network-queryable store by using the
//! `dht_lookup` gossip topic. On a local miss, a `dht_query` is published;
//! peers that hold the key respond with `dht_response`. Responses are
//! cached locally and delivered via oneshot channels to waiting callers.

use crate::network::NetworkSubsystem;
use crate::services::dht::DhtNodeService;
use crate::services::ServiceContainer;
use ephemera_dht::storage::DhtStorage;
use ephemera_gossip::{GossipTopic, TopicSubscription};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// DHT query wire message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtQueryMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub key: String,
    pub requester: String,
    pub query_id: String,
}

/// DHT response wire message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DhtResponseMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub key: String,
    pub value: Value,
    pub responder: String,
    pub query_id: String,
}

const DHT_QUERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Pending DHT queries: maps query_id -> oneshot sender.
pub type PendingDhtQueries = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// Create a new pending queries map.
pub fn new_pending_queries() -> PendingDhtQueries {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Decode a hex string to a 32-byte array, returning None on failure.
fn decode_key(hex_str: &str) -> Option<[u8; 32]> {
    let b = hex::decode(hex_str).ok()?;
    if b.len() != 32 { return None; }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&b);
    Some(arr)
}

/// Background loop: process DHT query/response messages from gossip.
pub async fn dht_query_ingest_loop_services(
    mut subscription: TopicSubscription,
    services: Arc<ServiceContainer>,
    network: Arc<NetworkSubsystem>,
    pending_queries: PendingDhtQueries,
    our_node_id: String,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            msg = subscription.recv() => {
                let msg = match msg {
                    Some(m) => m,
                    None => break,
                };
                let payload: Value = match serde_json::from_slice(&msg.payload) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match payload.get("type").and_then(|t| t.as_str()) {
                    Some("dht_query") => {
                        handle_query(&payload, &services.dht_storage, &network, &our_node_id).await;
                    }
                    Some("dht_response") => {
                        handle_response(&payload, &services.dht_storage, &services.identity, &pending_queries);
                    }
                    _ => {}
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("dht query ingest: shutdown");
                break;
            }
        }
    }
}

/// Check local DHT for the queried key; if found, publish a response.
async fn handle_query(
    payload: &Value,
    dht_storage: &Mutex<DhtStorage>,
    network: &NetworkSubsystem,
    our_node_id: &str,
) {
    let key_hex = match payload.get("key").and_then(|v| v.as_str()) { Some(k) => k, None => return };
    let requester = match payload.get("requester").and_then(|v| v.as_str()) { Some(r) => r, None => return };
    let query_id = match payload.get("query_id").and_then(|v| v.as_str()) { Some(q) => q, None => return };

    if requester == our_node_id { return; }

    let key_bytes = match decode_key(key_hex) { Some(k) => k, None => return };

    let value = match DhtNodeService::get(&key_bytes, dht_storage) {
        Ok(Some(v)) => v,
        _ => return,
    };

    tracing::debug!(key = %key_hex, query_id = %query_id, "responding to DHT query");

    let response = DhtResponseMessage {
        msg_type: "dht_response".to_string(),
        key: key_hex.to_string(),
        value,
        responder: our_node_id.to_string(),
        query_id: query_id.to_string(),
    };

    let bytes = match serde_json::to_vec(&response) { Ok(b) => b, Err(_) => return };
    let topic = GossipTopic::dht_lookup();
    if let Err(e) = network.publish(&topic, bytes).await {
        tracing::warn!(error = %e, "dht query: publish response failed");
    }
}

/// Store the response value locally and wake the pending query waiter.
fn handle_response(
    payload: &Value,
    dht_storage: &Mutex<DhtStorage>,
    identity: &crate::services::identity::IdentityService,
    pending_queries: &PendingDhtQueries,
) {
    let key_hex = match payload.get("key").and_then(|v| v.as_str()) { Some(k) => k, None => return };
    let value = match payload.get("value") { Some(v) => v.clone(), None => return };
    let query_id = match payload.get("query_id").and_then(|v| v.as_str()) { Some(q) => q, None => return };

    tracing::debug!(key = %key_hex, query_id = %query_id, "received DHT response");

    let key_bytes = match decode_key(key_hex) { Some(k) => k, None => return };
    let value_bytes = match serde_json::to_vec(&value) { Ok(b) => b, Err(_) => return };

    // Cache in local DHT (best-effort).
    let _ = DhtNodeService::put(
        key_bytes, value_bytes,
        ephemera_dht::MAX_TTL_SECONDS,
        ephemera_dht::DhtRecordType::Profile,
        identity, dht_storage,
    );

    // Wake pending waiter.
    if let Ok(mut pending) = pending_queries.lock() {
        if let Some(tx) = pending.remove(query_id) {
            let _ = tx.send(value);
        }
    }
}

/// Query the network DHT for a key. Checks local first; on miss, publishes
/// a query via gossip and waits up to 5 seconds for a response.
pub async fn query_network_dht(
    key: &[u8; 32],
    services: &Arc<ServiceContainer>,
) -> Result<Option<Value>, String> {
    query_network_dht_inner(key, services, false).await
}

/// Query the network DHT, optionally skipping the local cache.
///
/// When `skip_local` is true, the local DHT is not checked first — useful
/// when the caller wants the freshest data from the network (e.g. profile
/// lookups where the owner may have updated their profile).
pub async fn query_network_dht_fresh(
    key: &[u8; 32],
    services: &Arc<ServiceContainer>,
) -> Result<Option<Value>, String> {
    query_network_dht_inner(key, services, true).await
}

async fn query_network_dht_inner(
    key: &[u8; 32],
    services: &Arc<ServiceContainer>,
    skip_local: bool,
) -> Result<Option<Value>, String> {
    // 1. Check local DHT (unless caller wants fresh network data).
    if !skip_local {
        if let Ok(Some(val)) = DhtNodeService::get(key, &services.dht_storage) {
            return Ok(Some(val));
        }
    }

    // 2. Get network; bail if unavailable or no peers.
    let network = {
        let guard = services.network.lock().map_err(|e| format!("lock: {e}"))?;
        match guard.as_ref() {
            Some(net) => Arc::clone(net),
            None => return Ok(None),
        }
    };
    if network.peer_count() == 0 { return Ok(None); }

    // 3. Generate unique query ID and register oneshot.
    let mut id_bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut id_bytes);
    let query_id = hex::encode(id_bytes);
    let (tx, rx) = oneshot::channel();
    {
        let mut pending = services.pending_dht_queries.lock()
            .map_err(|e| format!("lock: {e}"))?;
        pending.insert(query_id.clone(), tx);
    }

    // 4. Publish query.
    let key_hex = hex::encode(key);
    let query = DhtQueryMessage {
        msg_type: "dht_query".to_string(),
        key: key_hex.clone(),
        requester: hex::encode(network.local_id().as_bytes()),
        query_id: query_id.clone(),
    };
    let query_bytes = serde_json::to_vec(&query)
        .map_err(|e| format!("serialize: {e}"))?;
    let topic = GossipTopic::dht_lookup();
    network.publish(&topic, query_bytes).await
        .map_err(|e| format!("publish dht query: {e}"))?;

    tracing::debug!(key = %key_hex, query_id = %query_id, "published DHT query");

    // 5. Wait for response with timeout.
    match tokio::time::timeout(DHT_QUERY_TIMEOUT, rx).await {
        Ok(Ok(value)) => Ok(Some(value)),
        Ok(Err(_)) => Ok(None),
        Err(_) => {
            if let Ok(mut p) = services.pending_dht_queries.lock() {
                p.remove(&query_id);
            }
            tracing::debug!(key = %key_hex, "DHT query timed out (5s)");
            Ok(None)
        }
    }
}
