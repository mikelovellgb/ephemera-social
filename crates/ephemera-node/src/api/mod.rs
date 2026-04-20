//! JSON-RPC 2.0 API handler. Dispatches identity, handle, post, feed,
//! social, message, profile, moderation, media, and meta RPC methods.

mod content;
pub mod dht;
mod meta;
pub mod network;
mod social;

use crate::debug_log::DebugLogHandle;
use crate::rpc::{error_codes, JsonRpcError, Router};
use crate::services::ServiceContainer;
use serde_json::Value;
use std::sync::Arc;

/// Build a fully-wired JSON-RPC router with all API methods registered.
///
/// The returned [`Router`] is ready to dispatch incoming requests.
pub fn build_router(services: Arc<ServiceContainer>) -> Router {
    build_router_with_network(services, None)
}

/// Build a router that includes `network.*` methods.
///
/// All network status reads go through `services.network` (the Mutex) so
/// handlers always see the current subsystem after a TCP-to-Iroh upgrade.
/// When `debug_log` is `Some`, the `meta.debug_log` endpoint is registered so
/// the frontend debug console can fetch captured log entries and network status.
pub fn build_router_with_network(
    services: Arc<ServiceContainer>,
    debug_log: Option<DebugLogHandle>,
) -> Router {
    let mut router = Router::new();

    register_identity(&mut router, &services);
    register_handles(&mut router, &services);
    content::register_posts(&mut router, &services);
    content::register_media(&mut router, &services);
    content::register_feed(&mut router, &services);
    social::register_social(&mut router, &services);
    social::register_messages(&mut router, &services);
    social::register_profiles(&mut router, &services);
    social::register_moderation(&mut router, &services);
    social::register_topics(&mut router, &services);
    social::register_groups(&mut router, &services);
    social::register_group_chats(&mut router, &services);
    social::register_mentions(&mut router, &services);
    social::register_notifications(&mut router, &services);
    content::register_discover(&mut router, &services);
    dht::register_dht(&mut router, &services);

    // Register network methods that read from services.network (the Mutex).
    // This ensures they always see the CURRENT network subsystem, even after
    // an upgrade from TCP to Iroh. Previously we passed a direct Arc which
    // became stale after upgrade.
    network::register_network_dynamic(&mut router, &services);

    // Collect all method names for meta.capabilities, then register meta
    // (only add names for methods not yet registered above).
    let mut method_names = router.method_names();
    for name in [
        "meta.capabilities",
        "meta.status",
        "meta.set_transport_tier",
        "meta.debug_log",
    ] {
        if !method_names.contains(&name.to_string()) {
            method_names.push(name.to_string());
        }
    }
    // Network method names are always registered now (real or stubs).
    for name in [
        "network.info",
        "network.connect",
        "network.peers",
        "network.disconnect",
        "network.status",
    ] {
        if !method_names.contains(&name.to_string()) {
            method_names.push(name.to_string());
        }
    }
    method_names.sort();
    meta::register_meta(&mut router, &services, method_names, debug_log);

    router
}

// ---------------------------------------------------------------------------
// identity.* namespace
// ---------------------------------------------------------------------------

fn register_identity(router: &mut Router, services: &Arc<ServiceContainer>) {
    let svc = Arc::clone(services);
    router.register("identity.create", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let passphrase = extract_str(&params, "passphrase")?;
            let result = svc.identity
                .create(&passphrase)
                .await
                .map_err(internal_error)?;

            // Start the Iroh network now that we have an identity.
            if let Err(e) = svc.start_network().await {
                tracing::warn!(error = %e, "failed to start network after identity create");
            }

            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.unlock", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let passphrase = extract_str(&params, "passphrase")?;
            let remember = params.get("remember").and_then(|v| v.as_bool()).unwrap_or(false);
            let result = svc.identity
                .unlock(&passphrase, remember)
                .await
                .map_err(internal_error)?;

            // Start the Iroh network now that identity is unlocked.
            if let Err(e) = svc.start_network().await {
                tracing::warn!(error = %e, "failed to start network after unlock");
            }

            // Publish the local profile to DHT on unlock so connected peers
            // can discover our display_name. Best-effort, never fail unlock.
            {
                use crate::services::dht::DhtNodeService;
                if let Ok(signing_kp) = svc.identity.get_signing_keypair() {
                    let pubkey_hex = hex::encode(signing_kp.public_key().as_bytes());
                    let pubkey_bytes = signing_kp.public_key().as_bytes().to_vec();

                    // Read profile from local DB.
                    let profile_data: Option<(Option<String>, Option<String>)> =
                        if let Ok(db) = svc.metadata_db.lock() {
                            db.conn()
                                .query_row(
                                    "SELECT display_name, bio FROM profiles WHERE pubkey = ?1",
                                    rusqlite::params![pubkey_bytes],
                                    |row| Ok((row.get(0)?, row.get(1)?)),
                                )
                                .ok()
                        } else {
                            None
                        };

                    if let Some((name, bio)) = profile_data {
                        if name.is_some() || bio.is_some() {
                            let profile_json = serde_json::json!({
                                "pubkey": pubkey_hex,
                                "display_name": name,
                                "bio": bio,
                            });
                            if let Err(e) = DhtNodeService::store_profile(
                                &pubkey_hex,
                                &profile_json,
                                &svc.identity,
                                &svc.dht_storage,
                            ) {
                                tracing::warn!(error = %e, "failed to publish profile to DHT on unlock");
                            } else {
                                tracing::info!(pubkey = %pubkey_hex, "profile published to DHT on unlock");
                            }
                        }
                    }
                }
            }

            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.lock", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let forget = params.get("forget").and_then(|v| v.as_bool()).unwrap_or(false);
            svc.identity.lock(forget).await.map_err(internal_error)
        }
    });

    // Auto-unlock using cached session key (for "remember me" feature).
    let svc = Arc::clone(services);
    router.register("identity.auto_unlock", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let result = svc.identity
                .auto_unlock()
                .await
                .map_err(internal_error)?;

            // If auto-unlock succeeded, start the network.
            if result.get("auto_unlocked").and_then(|v| v.as_bool()) == Some(true) {
                if let Err(e) = svc.start_network().await {
                    tracing::warn!(error = %e, "failed to start network after auto-unlock");
                }
            }

            Ok(result)
        }
    });

    // Check if a cached session key exists.
    let svc = Arc::clone(services);
    router.register("identity.has_session", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            Ok(serde_json::json!({ "has_session": svc.identity.has_session() }))
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.has_keystore", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let exists = svc.identity.has_keystore();
            Ok(serde_json::json!({ "exists": exists }))
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.get_active", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            let mut result = svc.identity.get_active().await.map_err(internal_error)?;
            // Enrich with display_name, bio, avatar_url from profiles table.
            if let Some(pubkey_hex) = result.get("pubkey").and_then(|v| v.as_str()) {
                if let Ok(pubkey_bytes) = hex::decode(pubkey_hex) {
                    if let Ok(db) = svc.metadata_db.lock() {
                        let profile_row: Option<(Option<String>, Option<String>, Option<Vec<u8>>)> = db
                            .conn()
                            .query_row(
                                "SELECT display_name, bio, avatar_cid FROM profiles WHERE pubkey = ?1",
                                rusqlite::params![pubkey_bytes],
                                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                            )
                            .ok();
                        if let Some(obj) = result.as_object_mut() {
                            if let Some((display_name, bio, avatar_cid)) = profile_row {
                                obj.insert(
                                    "display_name".to_string(),
                                    display_name
                                        .map(Value::String)
                                        .unwrap_or(Value::Null),
                                );
                                obj.insert(
                                    "bio".to_string(),
                                    bio.map(Value::String).unwrap_or(Value::Null),
                                );
                                let avatar_url = avatar_cid
                                    .as_ref()
                                    .map(|cid| format!("/media/{}", hex::encode(cid)));
                                obj.insert(
                                    "avatar_url".to_string(),
                                    avatar_url
                                        .map(Value::String)
                                        .unwrap_or(Value::Null),
                                );
                            } else {
                                obj.insert("display_name".to_string(), Value::Null);
                                obj.insert("bio".to_string(), Value::Null);
                                obj.insert("avatar_url".to_string(), Value::Null);
                            }
                        }
                    }
                }
            }
            // Enrich with @handle from the handle registry.
            if let Ok(handle_result) =
                crate::services::HandleService::my_handle(&svc.identity, &svc.handle_registry)
            {
                if let Some(obj) = result.as_object_mut() {
                    if handle_result.is_null() {
                        obj.insert("handle".to_string(), Value::Null);
                    } else {
                        obj.insert(
                            "handle".to_string(),
                            handle_result
                                .get("handle")
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                    }
                }
            }
            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.list_pseudonyms", move |_params| {
        let svc = Arc::clone(&svc);
        async move { svc.identity.list_pseudonyms().await.map_err(internal_error) }
    });

    let svc = Arc::clone(services);
    router.register("identity.switch_pseudonym", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let index = extract_u64(&params, "index")?;
            svc.identity
                .switch_pseudonym(index)
                .await
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.backup_mnemonic", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let passphrase = extract_str(&params, "passphrase")?;
            svc.identity
                .backup_mnemonic(&passphrase)
                .await
                .map_err(internal_error)
        }
    });

    // Multi-device: export
    let svc = Arc::clone(services);
    router.register("identity.export_mnemonic", move |_params| {
        let svc = Arc::clone(&svc);
        async move { svc.identity.export_mnemonic().await.map_err(internal_error) }
    });
    let svc = Arc::clone(services);
    router.register("identity.export_qr", move |_params| {
        let svc = Arc::clone(&svc);
        async move { svc.identity.export_qr().await.map_err(internal_error) }
    });
    let svc = Arc::clone(services);
    router.register("identity.invite_qr", move |_params| {
        let svc = Arc::clone(&svc);
        async move { svc.identity.invite_qr().await.map_err(internal_error) }
    });
    let svc = Arc::clone(services);
    router.register("identity.export_backup", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let passphrase = extract_str(&params, "passphrase")?;
            svc.identity.export_backup(&passphrase).await.map_err(internal_error)
        }
    });

    // Multi-device: import
    let svc = Arc::clone(services);
    router.register("identity.import_mnemonic", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let words_val = params.get("words").ok_or_else(|| JsonRpcError {
                code: error_codes::INVALID_PARAMS,
                message: "missing parameter: words".into(),
                data: None,
            })?;
            let words: Vec<String> = words_val
                .as_array()
                .ok_or_else(|| JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: "words must be an array of strings".into(),
                    data: None,
                })?
                .iter()
                .map(|v| v.as_str().map(String::from).ok_or_else(|| JsonRpcError {
                    code: error_codes::INVALID_PARAMS,
                    message: "each word must be a string".into(),
                    data: None,
                }))
                .collect::<Result<Vec<_>, _>>()?;
            let passphrase = extract_str(&params, "passphrase")?;
            let result = svc.identity.import_mnemonic(&words, &passphrase).await.map_err(internal_error)?;

            if let Err(e) = svc.start_network().await {
                tracing::warn!(error = %e, "failed to start network after mnemonic import");
            }

            Ok(result)
        }
    });
    let svc = Arc::clone(services);
    router.register("identity.import_backup", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let data = extract_str(&params, "data")?;
            let backup_passphrase = extract_str(&params, "backup_passphrase")?;
            let new_passphrase = extract_str(&params, "new_passphrase")?;
            let result = svc.identity
                .import_backup(&data, &backup_passphrase, &new_passphrase)
                .await
                .map_err(internal_error)?;

            if let Err(e) = svc.start_network().await {
                tracing::warn!(error = %e, "failed to start network after backup import");
            }

            Ok(result)
        }
    });

    // Import from QR code (hex-encoded QR payload)
    let svc = Arc::clone(services);
    router.register("identity.import_qr", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let qr_hex = extract_str(&params, "qr_hex")?;
            let passphrase = extract_str(&params, "passphrase")?;
            let result = svc.identity
                .import_qr(&qr_hex, &passphrase)
                .await
                .map_err(internal_error)?;

            if let Err(e) = svc.start_network().await {
                tracing::warn!(error = %e, "failed to start network after QR import");
            }

            Ok(result)
        }
    });

    // Multi-device: device management
    let svc = Arc::clone(services);
    router.register("identity.register_device", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            let platform = extract_str(&params, "platform")?;
            svc.identity.register_device(&name, &platform).await.map_err(internal_error)
        }
    });
    let svc = Arc::clone(services);
    router.register("identity.list_devices", move |_params| {
        let svc = Arc::clone(&svc);
        async move { svc.identity.list_devices().await.map_err(internal_error) }
    });
    let svc = Arc::clone(services);
    router.register("identity.revoke_device", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let device_id = extract_str(&params, "device_id")?;
            svc.identity.revoke_device(&device_id).await.map_err(internal_error)
        }
    });
}

// ---------------------------------------------------------------------------
// Handle RPC methods (identity.register_handle, lookup_handle, etc.)
// ---------------------------------------------------------------------------

fn register_handles(router: &mut Router, services: &Arc<ServiceContainer>) {
    use crate::services::HandleService;

    let svc = Arc::clone(services);
    router.register("identity.register_handle", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            // PoW computation is CPU-intensive and can take seconds to minutes.
            // Run it on a blocking thread so it doesn't stall the async runtime
            // (which would block ALL other RPC calls).
            let svc_inner = Arc::clone(&svc);
            let name_inner = name.clone();
            let result = tokio::task::spawn_blocking(move || {
                HandleService::register_and_publish(
                    &name_inner,
                    &svc_inner.identity,
                    &svc_inner.handle_registry,
                    &svc_inner.dht_storage,
                )
            })
            .await
            .map_err(|e| JsonRpcError {
                code: error_codes::POW_FAILED,
                message: format!("handle registration task failed: {e}"),
                data: None,
            })?
            .map_err(internal_error)?;
            Ok(result)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.lookup_handle", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            HandleService::lookup_with_network_dht(
                &name, &svc.handle_registry, &svc,
            )
            .await
            .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.renew_handle", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            HandleService::renew(&name, &svc.identity, &svc.handle_registry)
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.release_handle", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            HandleService::release(&name, &svc.identity, &svc.handle_registry)
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.my_handle", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            HandleService::my_handle(&svc.identity, &svc.handle_registry)
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.check_handle_status", move |_params| {
        let svc = Arc::clone(&svc);
        async move {
            HandleService::check_handle_status(&svc.identity, &svc.handle_registry)
                .map_err(internal_error)
        }
    });

    let svc = Arc::clone(services);
    router.register("identity.check_handle_available", move |params| {
        let svc = Arc::clone(&svc);
        async move {
            let name = extract_str(&params, "name")?;
            // Check local registry, local DHT, then network DHT.
            let result = HandleService::lookup_with_network_dht(
                &name, &svc.handle_registry, &svc,
            )
            .await
            .map_err(internal_error)?;

            if result.is_null() {
                Ok(serde_json::json!({
                    "available": true,
                    "name": name,
                }))
            } else {
                Ok(serde_json::json!({
                    "available": false,
                    "name": name,
                    "reason": "already taken",
                }))
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Parameter extraction helpers
// ---------------------------------------------------------------------------

/// Extract a required string parameter from the JSON-RPC params object.
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

/// Extract a required u64 parameter from the JSON-RPC params object.
fn extract_u64(params: &Value, key: &str) -> Result<u64, JsonRpcError> {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| JsonRpcError {
            code: error_codes::INVALID_PARAMS,
            message: format!("missing or invalid parameter: {key}"),
            data: None,
        })
}

/// Convert a service-level error string into a JSON-RPC internal error.
fn internal_error(msg: String) -> JsonRpcError {
    JsonRpcError {
        code: error_codes::INTERNAL_ERROR,
        message: msg,
        data: None,
    }
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
