//! JSON-RPC handlers for the `dht.*` namespace.
//!
//! Provides key-value storage and lookup operations on the local DHT,
//! enabling handle discovery, prekey retrieval, and profile replication
//! across the P2P network.

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

/// Extract an optional u64 parameter.
fn extract_opt_u64(params: &Value, key: &str) -> Option<u64> {
    params.get(key).and_then(|v| v.as_u64())
}

/// Convert a service error to a JSON-RPC internal error.
fn internal_error(msg: String) -> JsonRpcError {
    JsonRpcError {
        code: error_codes::INTERNAL_ERROR,
        message: msg,
        data: None,
    }
}

/// Register `dht.*` namespace methods on the router.
pub fn register_dht(router: &mut Router, services: &Arc<ServiceContainer>) {
    use crate::services::dht::DhtNodeService;
    use ephemera_dht::DhtRecordType;

    // dht.put(key, value, ttl_seconds) -- store a record
    let svc = Arc::clone(services);
    router.register("dht.put", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let key_hex = extract_str(&params, "key")?;
            let value_str = extract_str(&params, "value")?;
            let ttl = extract_opt_u64(&params, "ttl_seconds")
                .unwrap_or(30 * 24 * 60 * 60) as u32;

            let key_bytes = hex::decode(&key_hex).map_err(|e| JsonRpcError {
                code: error_codes::INVALID_PARAMS,
                message: format!("invalid key hex: {e}"),
                data: None,
            })?;
            if key_bytes.len() != 32 {
                return Err(JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: format!(
                        "key must be 32 bytes (64 hex chars), got {} bytes",
                        key_bytes.len()
                    ),
                    data: None,
                });
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&key_bytes);

            DhtNodeService::put(
                key,
                value_str.into_bytes(),
                ttl,
                DhtRecordType::ContentProvider,
                &svc.identity,
                &svc.dht_storage,
            )
            .map_err(internal_error)?;

            Ok(serde_json::json!({
                "ok": true,
                "key": key_hex,
            }))
        }
    });

    // dht.get(key) -- retrieve a record
    let svc = Arc::clone(services);
    router.register("dht.get", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let key_hex = extract_str(&params, "key")?;
            let key_bytes = hex::decode(&key_hex).map_err(|e| JsonRpcError {
                code: error_codes::INVALID_PARAMS,
                message: format!("invalid key hex: {e}"),
                data: None,
            })?;
            if key_bytes.len() != 32 {
                return Err(JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: format!(
                        "key must be 32 bytes (64 hex chars), got {} bytes",
                        key_bytes.len()
                    ),
                    data: None,
                });
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&key_bytes);

            let result = DhtNodeService::get(&key, &svc.dht_storage)
                .map_err(internal_error)?;

            match result {
                Some(val) => Ok(serde_json::json!({
                    "found": true,
                    "key": key_hex,
                    "value": val,
                })),
                None => Ok(serde_json::json!({
                    "found": false,
                    "key": key_hex,
                })),
            }
        }
    });

    // dht.lookup_handle(name) -- find a handle via DHT
    let svc = Arc::clone(services);
    router.register("dht.lookup_handle", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            let result = DhtNodeService::lookup_handle(&name, &svc.dht_storage)
                .map_err(internal_error)?;
            match result {
                Some(val) => Ok(val),
                None => Ok(Value::Null),
            }
        }
    });

    // dht.lookup_prekey(pubkey) -- find a prekey bundle via DHT
    let svc = Arc::clone(services);
    router.register("dht.lookup_prekey", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let pubkey = extract_str(&params, "pubkey")?;
            let result = DhtNodeService::lookup_prekey(&pubkey, &svc.dht_storage)
                .map_err(internal_error)?;
            match result {
                Some(val) => Ok(val),
                None => Ok(Value::Null),
            }
        }
    });

    // dht.lookup_profile(pubkey) -- find a profile via DHT
    let svc = Arc::clone(services);
    router.register("dht.lookup_profile", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let pubkey = extract_str(&params, "pubkey")?;
            let result = DhtNodeService::lookup_profile(&pubkey, &svc.dht_storage)
                .map_err(internal_error)?;
            match result {
                Some(val) => Ok(val),
                None => Ok(Value::Null),
            }
        }
    });

    // dht.status() -- return DHT storage stats
    let svc = Arc::clone(services);
    router.register("dht.status", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let count = DhtNodeService::record_count(&svc.dht_storage)
                .map_err(internal_error)?;
            let routing_entries = {
                let rt = svc.dht_routing.lock()
                    .map_err(|e| internal_error(format!("lock: {e}")))?;
                rt.total_entries()
            };
            Ok(serde_json::json!({
                "records": count,
                "routing_table_entries": routing_entries,
                "max_records": svc.dht_config.max_records,
                "k": svc.dht_config.k,
            }))
        }
    });
}
