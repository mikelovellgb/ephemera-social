//! Handle registration, lookup, and renewal service.
//!
//! Wires the [`HandleRegistry`] from `ephemera-social` into the node,
//! providing RPC-accessible methods for human-readable `@username`
//! management.

use super::dht::DhtNodeService;
use super::identity::IdentityService;
use ephemera_crypto::identity::PseudonymIdentity;
use ephemera_dht::storage::DhtStorage;
use ephemera_events::{Event, EventBus};
use ephemera_social::handle_validation::PowDifficulty;
use ephemera_social::{Handle, HandleRegistry, InsertOutcome};
use serde_json::Value;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Return the current wall-clock time as Unix seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

/// Handle management service.
///
/// All methods are stateless -- the state lives in the `HandleRegistry`
/// stored on the `ServiceContainer`.
pub struct HandleService;

impl HandleService {
    /// Register a new handle for the current identity.
    ///
    /// Performs PoW computation (difficulty scales inversely with handle
    /// length), signs the registration, stores the record in the local
    /// registry, and publishes it to the DHT for network discovery.
    pub fn register(
        name: &str,
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
    ) -> Result<Value, String> {
        let pseudonym = get_pseudonym_identity(identity)?;
        let now = now_secs();

        let mut reg = registry.lock().map_err(|e| format!("lock: {e}"))?;
        let handle = reg
            .register(name, &pseudonym, now)
            .map_err(|e| format!("register handle: {e}"))?;

        Ok(serde_json::json!({
            "handle": format!("@{}", handle.name),
            "owner": hex::encode(handle.owner.as_bytes()),
            "registered_at": handle.registered_at,
            "expires_at": handle.expires_at,
        }))
    }

    /// Register a new handle and also publish it to the DHT.
    ///
    /// This is the full registration path used by the node API. The DHT
    /// publication makes the handle discoverable by other nodes.
    pub fn register_and_publish(
        name: &str,
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Value, String> {
        let result = Self::register(name, identity, registry)?;

        // Best-effort DHT publication: log but don't fail the registration.
        let owner = result["owner"].as_str().unwrap_or_default().to_string();
        let registered_at = result["registered_at"].as_u64().unwrap_or(0);
        let expires_at = result["expires_at"].as_u64().unwrap_or(0);

        if let Err(e) = DhtNodeService::store_handle(
            name, &owner, registered_at, expires_at, identity, dht_storage,
        ) {
            tracing::warn!(handle = name, error = %e, "failed to publish handle to DHT");
        } else {
            tracing::info!(handle = name, "handle published to DHT");
        }

        Ok(result)
    }

    /// Register a handle with explicit difficulty (for testing).
    pub fn register_with_difficulty(
        name: &str,
        difficulty: PowDifficulty,
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
    ) -> Result<Value, String> {
        let pseudonym = get_pseudonym_identity(identity)?;
        let now = now_secs();

        let mut reg = registry.lock().map_err(|e| format!("lock: {e}"))?;
        let handle = reg
            .register_with_difficulty(name, &pseudonym, difficulty, now)
            .map_err(|e| format!("register handle: {e}"))?;

        Ok(serde_json::json!({
            "handle": format!("@{}", handle.name),
            "owner": hex::encode(handle.owner.as_bytes()),
            "registered_at": handle.registered_at,
            "expires_at": handle.expires_at,
        }))
    }

    /// Register with explicit difficulty and publish to DHT.
    pub fn register_with_difficulty_and_publish(
        name: &str,
        difficulty: PowDifficulty,
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Value, String> {
        let result = Self::register_with_difficulty(name, difficulty, identity, registry)?;

        let owner = result["owner"].as_str().unwrap_or_default().to_string();
        let registered_at = result["registered_at"].as_u64().unwrap_or(0);
        let expires_at = result["expires_at"].as_u64().unwrap_or(0);

        if let Err(e) = DhtNodeService::store_handle(
            name, &owner, registered_at, expires_at, identity, dht_storage,
        ) {
            tracing::warn!(handle = name, error = %e, "failed to publish handle to DHT");
        }

        Ok(result)
    }

    /// Look up a handle by name.
    ///
    /// Returns the handle record if found and not expired, or null.
    pub fn lookup(
        name: &str,
        registry: &Mutex<HandleRegistry>,
    ) -> Result<Value, String> {
        let reg = registry.lock().map_err(|e| format!("lock: {e}"))?;
        match reg.lookup(name) {
            Some(handle) if !handle.is_expired() => Ok(serde_json::json!({
                "handle": format!("@{}", handle.name),
                "owner": hex::encode(handle.owner.as_bytes()),
                "registered_at": handle.registered_at,
                "expires_at": handle.expires_at,
            })),
            _ => Ok(Value::Null),
        }
    }

    /// Look up a handle by name, falling back to the DHT if not in the
    /// local registry. This enables discovery of handles registered on
    /// other nodes.
    pub fn lookup_with_dht(
        name: &str,
        registry: &Mutex<HandleRegistry>,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Value, String> {
        // Try local registry first.
        let result = Self::lookup(name, registry)?;
        if !result.is_null() {
            return Ok(result);
        }

        // Fall back to DHT.
        match DhtNodeService::lookup_handle(name, dht_storage)? {
            Some(val) => Ok(val),
            None => Ok(Value::Null),
        }
    }

    /// Look up a handle by name, querying the network DHT if not found
    /// locally. This is the async version of `lookup_with_dht` that performs
    /// a gossip-based DHT query to remote peers.
    pub async fn lookup_with_network_dht(
        name: &str,
        registry: &Mutex<HandleRegistry>,
        services: &std::sync::Arc<crate::services::ServiceContainer>,
    ) -> Result<Value, String> {
        // Try local registry first.
        let result = Self::lookup(name, registry)?;
        if !result.is_null() {
            return Ok(result);
        }

        // Try local DHT.
        match DhtNodeService::lookup_handle(name, &services.dht_storage)? {
            Some(val) => return Ok(val),
            None => {}
        }

        // Query the network DHT.
        let key = super::dht::dht_key("handle", name);
        match crate::dht_query::query_network_dht(&key, services).await? {
            Some(val) => Ok(val),
            None => Ok(Value::Null),
        }
    }

    /// Look up the handle owned by the current identity (reverse lookup).
    pub fn my_handle(
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
    ) -> Result<Value, String> {
        let signing_kp = identity.get_signing_keypair()?;
        let pubkey = signing_kp.public_key();

        let reg = registry.lock().map_err(|e| format!("lock: {e}"))?;
        match reg.lookup_by_owner(&pubkey) {
            Some(handle) if !handle.is_expired() => Ok(serde_json::json!({
                "handle": format!("@{}", handle.name),
                "registered_at": handle.registered_at,
                "expires_at": handle.expires_at,
            })),
            _ => Ok(Value::Null),
        }
    }

    /// Renew an existing handle, extending its expiry by another 90 days.
    pub fn renew(
        name: &str,
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
    ) -> Result<Value, String> {
        let pseudonym = get_pseudonym_identity(identity)?;
        let now = now_secs();

        let mut reg = registry.lock().map_err(|e| format!("lock: {e}"))?;
        reg.renew(name, &pseudonym, now)
            .map_err(|e| format!("renew handle: {e}"))?;

        Ok(serde_json::json!({ "renewed": true, "handle": format!("@{}", name) }))
    }

    /// Process a handle record received from the network (gossip / DHT sync).
    ///
    /// Validates the record, resolves conflicts, and emits a
    /// `HandleConflictLost` event if our own handle was displaced.
    pub fn receive_remote_handle(
        handle: Handle,
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
        event_bus: &EventBus,
    ) -> Result<Value, String> {
        let now = now_secs();

        // Validate the incoming record.
        HandleRegistry::validate(&handle, now).map_err(|e| format!("invalid handle: {e}"))?;

        let mut reg = registry.lock().map_err(|e| format!("lock: {e}"))?;
        let outcome = reg.insert_validated(handle.clone(), now);

        match &outcome {
            InsertOutcome::Replaced { displaced_owner } => {
                // Check if the displaced owner is US.
                if let Ok(kp) = identity.get_signing_keypair() {
                    if kp.public_key() == *displaced_owner {
                        tracing::warn!(
                            handle = %handle.name,
                            "our handle was displaced by a conflicting registration"
                        );
                        event_bus.emit(Event::HandleConflictLost {
                            handle_name: handle.name.clone(),
                            new_owner: handle.owner,
                        });
                    }
                }
            }
            InsertOutcome::Rejected => {
                tracing::debug!(
                    handle = %handle.name,
                    "rejected incoming handle: existing registration wins"
                );
            }
            InsertOutcome::Inserted => {
                tracing::debug!(
                    handle = %handle.name,
                    "accepted incoming handle registration"
                );
            }
        }

        let accepted = outcome != InsertOutcome::Rejected;
        Ok(serde_json::json!({
            "accepted": accepted,
            "handle": format!("@{}", handle.name),
        }))
    }

    /// Check whether our handle is still valid (not displaced by a conflict).
    ///
    /// Returns the current handle info, or null if we no longer own one.
    /// The frontend polls this to detect conflict-induced revocations.
    pub fn check_handle_status(
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
    ) -> Result<Value, String> {
        let kp = identity.get_signing_keypair()?;
        let pubkey = kp.public_key();

        let reg = registry.lock().map_err(|e| format!("lock: {e}"))?;
        match reg.lookup_by_owner(&pubkey) {
            Some(handle) if !handle.is_expired() => Ok(serde_json::json!({
                "active": true,
                "handle": format!("@{}", handle.name),
                "registered_at": handle.registered_at,
                "expires_at": handle.expires_at,
            })),
            _ => Ok(serde_json::json!({
                "active": false,
            })),
        }
    }

    /// Release a handle, making it available after a 24-hour cooldown.
    pub fn release(
        name: &str,
        identity: &IdentityService,
        registry: &Mutex<HandleRegistry>,
    ) -> Result<Value, String> {
        let pseudonym = get_pseudonym_identity(identity)?;
        let now = now_secs();

        let mut reg = registry.lock().map_err(|e| format!("lock: {e}"))?;
        reg.release(name, &pseudonym, now)
            .map_err(|e| format!("release handle: {e}"))?;

        Ok(serde_json::json!({ "released": true, "handle": format!("@{}", name) }))
    }
}

/// Extract a `PseudonymIdentity` from the `IdentityService`'s current
/// unlocked master secret and active pseudonym index.
fn get_pseudonym_identity(
    identity: &IdentityService,
) -> Result<PseudonymIdentity, String> {
    let ms_guard = identity
        .master_secret
        .lock()
        .map_err(|e| format!("lock: {e}"))?;
    let master = ms_guard
        .as_ref()
        .ok_or("identity locked -- create or unlock first")?;
    let idx = *identity
        .active_index
        .lock()
        .map_err(|e| format!("lock: {e}"))?;
    PseudonymIdentity::derive(master, idx).map_err(|e| format!("derive pseudonym: {e}"))
}
