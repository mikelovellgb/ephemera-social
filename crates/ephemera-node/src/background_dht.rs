//! DHT background maintenance task.
//!
//! Periodically sweeps expired DHT records and re-publishes the local
//! user's handle, prekey, and profile so they remain discoverable as
//! records approach their TTL expiry.

use crate::services::dht::DhtNodeService;
use crate::services::ServiceContainer;
use std::sync::Arc;
use std::time::Duration;

/// DHT sweep interval: every 60 seconds.
const DHT_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// DHT republish interval: every 12 hours.
const DHT_REPUBLISH_INTERVAL: Duration = Duration::from_secs(12 * 3600);

/// DHT maintenance loop: periodically sweep expired records and
/// re-publish own records (handle, prekey, profile) for redundancy.
///
/// Runs on two intervals:
/// - Every 60 seconds: sweep expired DHT records
/// - Every 12 hours: re-publish own handle, prekey, and profile
pub async fn dht_maintenance_loop(
    services: Arc<ServiceContainer>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut sweep_interval = tokio::time::interval(DHT_SWEEP_INTERVAL);
    let mut republish_interval = tokio::time::interval(DHT_REPUBLISH_INTERVAL);

    loop {
        tokio::select! {
            _ = sweep_interval.tick() => {
                match DhtNodeService::sweep_expired(&services.dht_storage) {
                    Ok(removed) if removed > 0 => {
                        tracing::info!(removed, "DHT sweep: removed expired records");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "DHT sweep failed");
                    }
                }
            }
            _ = republish_interval.tick() => {
                republish_own_records(&services);
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("DHT maintenance loop shutting down");
                break;
            }
        }
    }
}

/// Re-publish the local user's handle, prekey bundle placeholder, and
/// profile to the DHT so they remain discoverable as records approach
/// their TTL expiry.
fn republish_own_records(services: &ServiceContainer) {
    // Only re-publish if the identity is unlocked.
    let signing_kp = match services.identity.get_signing_keypair() {
        Ok(kp) => kp,
        Err(_) => return, // identity locked, skip
    };
    let pubkey_hex = hex::encode(signing_kp.public_key().as_bytes());

    // Re-publish handle if one is registered.
    {
        let registry = match services.handle_registry.lock() {
            Ok(r) => r,
            Err(_) => return,
        };
        if let Some(handle) = registry.lookup_by_owner(&signing_kp.public_key()) {
            if !handle.is_expired() {
                let owner_hex = hex::encode(handle.owner.as_bytes());
                if let Err(e) = DhtNodeService::store_handle(
                    &handle.name,
                    &owner_hex,
                    handle.registered_at,
                    handle.expires_at,
                    &services.identity,
                    &services.dht_storage,
                ) {
                    tracing::warn!(error = %e, "DHT republish handle failed");
                } else {
                    tracing::debug!(handle = %handle.name, "DHT republished handle");
                }
            }
        }
    }

    // Re-publish profile from local DB (including avatar_cid).
    {
        let db = match services.metadata_db.lock() {
            Ok(d) => d,
            Err(_) => return,
        };
        let pubkey_bytes = signing_kp.public_key().as_bytes().to_vec();
        let profile: Option<(Option<String>, Option<String>, Option<Vec<u8>>)> = db
            .conn()
            .query_row(
                "SELECT display_name, bio, avatar_cid FROM profiles WHERE pubkey = ?1",
                rusqlite::params![pubkey_bytes],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        drop(db);

        if let Some((name, bio, avatar_cid)) = profile {
            let mut profile_json = serde_json::json!({
                "pubkey": pubkey_hex,
                "display_name": name,
                "bio": bio,
            });
            // Include avatar_cid so remote nodes know the avatar hash.
            if let Some(ref cid) = avatar_cid {
                if let Some(obj) = profile_json.as_object_mut() {
                    obj.insert(
                        "avatar_cid".to_string(),
                        serde_json::Value::String(hex::encode(cid)),
                    );
                }
            }
            if let Err(e) = DhtNodeService::store_profile(
                &pubkey_hex,
                &profile_json,
                &services.identity,
                &services.dht_storage,
            ) {
                tracing::warn!(error = %e, "DHT republish profile failed");
            } else {
                tracing::debug!("DHT republished profile");
            }
        }
    }

    tracing::debug!("DHT republish cycle complete");
}
