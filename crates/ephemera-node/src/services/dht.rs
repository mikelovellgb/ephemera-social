//! DHT integration service: store and retrieve handles, prekeys, and profiles.
//!
//! Bridges the `ephemera-dht` crate into the node's service layer, providing
//! typed put/get operations for the three record types that need DHT-backed
//! discovery: handles, prekey bundles, and user profiles.

use super::identity::IdentityService;
use ephemera_dht::storage::DhtStorage;
use ephemera_dht::{DhtRecord, DhtRecordType, MAX_TTL_SECONDS};
use serde_json::Value;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Handle TTL in the DHT: clamped to 30 days (the DHT maximum).
/// Handles have a 90-day TTL in the HandleRegistry but the DHT enforces
/// a 30-day max. The handle is re-published every 12 hours by the
/// background maintenance loop, keeping it alive across multiple DHT
/// TTL periods.
const HANDLE_TTL_SECONDS: u32 = MAX_TTL_SECONDS;

/// 30 days in seconds (prekey and profile TTL).
const DEFAULT_TTL_SECONDS: u32 = MAX_TTL_SECONDS;

/// Compute a DHT key by hashing a namespace prefix and identifier.
///
/// Returns `blake3("prefix:identifier")` truncated to 32 bytes.
pub fn dht_key(prefix: &str, identifier: &str) -> [u8; 32] {
    let input = format!("{prefix}:{identifier}");
    *blake3::hash(input.as_bytes()).as_bytes()
}

/// Compute a DHT key from a prefix and raw bytes (e.g., a public key).
pub fn dht_key_bytes(prefix: &str, bytes: &[u8]) -> [u8; 32] {
    let hex_id = hex::encode(bytes);
    dht_key(prefix, &hex_id)
}

/// Return the current wall-clock time as Unix seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_secs()
}

/// Build a `DhtRecord` without a real signature.
///
/// In a full implementation the record would be signed with the publisher's
/// Ed25519 key.  For the local-storage wiring we store a placeholder
/// signature; signature verification is enforced on *inbound* records from
/// the network, not on locally-authored ones.
fn build_record(
    key: [u8; 32],
    record_type: DhtRecordType,
    value: Vec<u8>,
    publisher: [u8; 32],
    ttl_seconds: u32,
    identity: &IdentityService,
) -> Result<DhtRecord, String> {
    let signing_kp = identity.get_signing_keypair()?;
    // Sign key||value to bind the record to the publisher.
    let mut sign_data = Vec::with_capacity(32 + value.len());
    sign_data.extend_from_slice(&key);
    sign_data.extend_from_slice(&value);
    let signature = signing_kp.sign(&sign_data);
    Ok(DhtRecord {
        key,
        record_type,
        value,
        publisher,
        timestamp: now_secs(),
        ttl_seconds,
        signature: signature.to_bytes().to_vec(),
    })
}

/// Node-level DHT operations.
pub struct DhtNodeService;

impl DhtNodeService {
    /// Store a handle record in the DHT.
    ///
    /// Key: `blake3("handle:<name>")`
    /// Value: JSON `{"name","owner","registered_at","expires_at"}`
    pub fn store_handle(
        handle_name: &str,
        owner_hex: &str,
        registered_at: u64,
        expires_at: u64,
        identity: &IdentityService,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<(), String> {
        let key = dht_key("handle", handle_name);
        let value = serde_json::to_vec(&serde_json::json!({
            "name": handle_name,
            "owner": owner_hex,
            "registered_at": registered_at,
            "expires_at": expires_at,
        }))
        .map_err(|e| format!("serialize handle: {e}"))?;

        let publisher = publisher_bytes(identity)?;
        let record = build_record(
            key,
            DhtRecordType::Profile, // reuse Profile type for handles
            value,
            publisher,
            HANDLE_TTL_SECONDS,
            identity,
        )?;

        let mut storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        storage.put(record).map_err(|e| format!("dht put: {e}"))
    }

    /// Look up a handle from the DHT.
    ///
    /// Returns the JSON record if found, or `None`.
    pub fn lookup_handle(
        handle_name: &str,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Option<Value>, String> {
        let key = dht_key("handle", handle_name);
        let storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        match storage.get(&key) {
            Some(record) => {
                let val: Value = serde_json::from_slice(&record.value)
                    .map_err(|e| format!("deserialize handle: {e}"))?;
                Ok(Some(val))
            }
            None => Ok(None),
        }
    }

    /// Store a prekey bundle in the DHT.
    ///
    /// Key: `blake3("prekey:<pubkey_hex>")`
    /// Value: serialized bundle fields as JSON.
    pub fn store_prekey(
        pubkey_hex: &str,
        bundle_json: &Value,
        identity: &IdentityService,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<(), String> {
        let key = dht_key("prekey", pubkey_hex);
        let value = serde_json::to_vec(bundle_json)
            .map_err(|e| format!("serialize prekey: {e}"))?;

        let publisher = publisher_bytes(identity)?;
        let record = build_record(
            key,
            DhtRecordType::PrekeyBundle,
            value,
            publisher,
            DEFAULT_TTL_SECONDS,
            identity,
        )?;

        let mut storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        storage.put(record).map_err(|e| format!("dht put: {e}"))
    }

    /// Look up a prekey bundle from the DHT.
    pub fn lookup_prekey(
        pubkey_hex: &str,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Option<Value>, String> {
        let key = dht_key("prekey", pubkey_hex);
        let storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        match storage.get(&key) {
            Some(record) => {
                let val: Value = serde_json::from_slice(&record.value)
                    .map_err(|e| format!("deserialize prekey: {e}"))?;
                Ok(Some(val))
            }
            None => Ok(None),
        }
    }

    /// Store a profile in the DHT.
    ///
    /// Key: `blake3("profile:<pubkey_hex>")`
    /// Value: JSON profile data.
    pub fn store_profile(
        pubkey_hex: &str,
        profile_json: &Value,
        identity: &IdentityService,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<(), String> {
        let key = dht_key("profile", pubkey_hex);
        let value = serde_json::to_vec(profile_json)
            .map_err(|e| format!("serialize profile: {e}"))?;

        let publisher = publisher_bytes(identity)?;
        let record = build_record(
            key,
            DhtRecordType::Profile,
            value,
            publisher,
            DEFAULT_TTL_SECONDS,
            identity,
        )?;

        let mut storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        storage.put(record).map_err(|e| format!("dht put: {e}"))
    }

    /// Look up a profile from the DHT.
    pub fn lookup_profile(
        pubkey_hex: &str,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Option<Value>, String> {
        let key = dht_key("profile", pubkey_hex);
        let storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        match storage.get(&key) {
            Some(record) => {
                let val: Value = serde_json::from_slice(&record.value)
                    .map_err(|e| format!("deserialize profile: {e}"))?;
                Ok(Some(val))
            }
            None => Ok(None),
        }
    }

    /// Generic DHT put: store an arbitrary record.
    pub fn put(
        key: [u8; 32],
        value: Vec<u8>,
        ttl_seconds: u32,
        record_type: DhtRecordType,
        identity: &IdentityService,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<(), String> {
        let publisher = publisher_bytes(identity)?;
        let record = build_record(key, record_type, value, publisher, ttl_seconds, identity)?;
        let mut storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        storage.put(record).map_err(|e| format!("dht put: {e}"))
    }

    /// Generic DHT get: retrieve a record by key.
    pub fn get(
        key: &[u8; 32],
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Option<Value>, String> {
        let storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        match storage.get(key) {
            Some(record) => {
                // Try to parse as JSON; fall back to hex-encoded bytes.
                match serde_json::from_slice(&record.value) {
                    Ok(val) => Ok(Some(val)),
                    Err(_) => Ok(Some(serde_json::json!({
                        "raw_hex": hex::encode(&record.value),
                    }))),
                }
            }
            None => Ok(None),
        }
    }

    /// Sweep expired records from the DHT storage.
    ///
    /// Returns the number of records removed.
    pub fn sweep_expired(dht_storage: &Mutex<DhtStorage>) -> Result<usize, String> {
        let mut storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        Ok(storage.sweep_expired())
    }

    /// Get the number of records in the DHT storage.
    pub fn record_count(dht_storage: &Mutex<DhtStorage>) -> Result<usize, String> {
        let storage = dht_storage.lock().map_err(|e| format!("lock: {e}"))?;
        Ok(storage.len())
    }
}

/// Extract the 32-byte publisher public key from the identity service.
fn publisher_bytes(identity: &IdentityService) -> Result<[u8; 32], String> {
    let kp = identity.get_signing_keypair()?;
    Ok(*kp.public_key().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dht_key_is_deterministic() {
        let k1 = dht_key("handle", "alice");
        let k2 = dht_key("handle", "alice");
        assert_eq!(k1, k2);
    }

    #[test]
    fn dht_key_differs_by_prefix() {
        let k1 = dht_key("handle", "alice");
        let k2 = dht_key("prekey", "alice");
        assert_ne!(k1, k2);
    }

    #[test]
    fn dht_key_bytes_matches_hex() {
        let bytes = [0xABu8; 32];
        let k1 = dht_key_bytes("prekey", &bytes);
        let k2 = dht_key("prekey", &hex::encode(bytes));
        assert_eq!(k1, k2);
    }
}
