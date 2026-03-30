//! Background tasks spawned by `EphemeraNode::start()`.
//!
//! Each function runs as an independent tokio task with a shutdown watch
//! channel. They handle:
//! - Garbage collection (configurable interval)
//! - Epoch key rotation (daily) -- cryptographic shredding
//! - Handle registry GC (hourly) -- expire old handles
//! - Dead drop polling (every 5 minutes) -- check for offline messages
//! - Reputation decay (daily) -- decay accumulated reputation points

use crate::services::ServiceContainer;
use ephemera_events::{Event, EventBus};
use ephemera_message::DeadDropService;
use ephemera_social::ConnectionService;
use std::sync::Arc;
use std::time::Duration;

/// Epoch key rotation interval: once per day.
const EPOCH_ROTATION_INTERVAL: Duration = Duration::from_secs(24 * 3600);

/// Handle GC interval: once per hour.
const HANDLE_GC_INTERVAL: Duration = Duration::from_secs(3600);

/// Dead drop polling interval: every 5 minutes.
const DEAD_DROP_POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Reputation decay interval: once per day.
const REPUTATION_DECAY_INTERVAL: Duration = Duration::from_secs(24 * 3600);

/// Profile refresh interval: every 30 minutes.
const PROFILE_REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Garbage collection loop: periodically sweep expired posts and purge
/// old tombstones from the metadata database and content store.
pub async fn gc_loop(
    interval_secs: u64,
    services: Arc<ServiceContainer>,
    event_bus: EventBus,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval =
        tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                tracing::debug!("running garbage collection cycle");
                match services.run_gc() {
                    Ok((posts_deleted, tombstones_purged)) => {
                        if posts_deleted > 0 || tombstones_purged > 0 {
                            tracing::info!(
                                posts_deleted,
                                tombstones_purged,
                                "GC sweep completed"
                            );
                            event_bus.emit(Event::GarbageCollectionCompleted {
                                items_removed: posts_deleted + tombstones_purged,
                                bytes_freed: 0,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "GC sweep failed");
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("GC loop received shutdown signal");
                break;
            }
        }
    }
}

/// Rotate epoch keys and destroy expired ones (cryptographic shredding).
///
/// Runs daily. On each tick:
/// 1. If the identity is unlocked, initialize the epoch key manager
/// 2. Call `rotate()` to derive the current epoch key
/// 3. Call `destroy_expired_keys()` to shred keys older than 30 days
pub async fn epoch_rotation_loop(
    services: Arc<ServiceContainer>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(EPOCH_ROTATION_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Try to initialize the epoch key manager if not yet done.
                let _ = services.init_epoch_key_manager();

                let mut ekm_guard = match services.epoch_key_manager.lock() {
                    Ok(g) => g,
                    Err(_) => continue,
                };

                if let Some(ref mut ekm) = *ekm_guard {
                    match ekm.rotate() {
                        Ok(result) => {
                            tracing::info!(
                                new_epoch = result.new_epoch_id,
                                marked = result.marked_for_destruction.len(),
                                "epoch key rotation completed"
                            );
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "epoch key rotation failed");
                        }
                    }

                    let destruction = ekm.destroy_expired_keys();
                    if destruction.count > 0 {
                        tracing::info!(
                            destroyed = destruction.count,
                            "epoch keys destroyed (cryptographic shredding)"
                        );
                    }
                } else {
                    tracing::debug!(
                        "epoch rotation: identity not unlocked, skipping"
                    );
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("epoch rotation loop shutting down");
                break;
            }
        }
    }
}

/// Garbage-collect expired handles and completed cooldowns.
///
/// Runs hourly. Removes handles that have expired past their 90-day TTL
/// and clears cooldown entries for released handles.
pub async fn handle_gc_loop(
    services: Arc<ServiceContainer>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(HANDLE_GC_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_secs();

                let mut registry = match services.handle_registry.lock() {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                registry.gc(now);

                // Auto-renew our own handle if it expires within 7 days.
                // We extend the expiry by 90 days from now, avoiding the full
                // registration PoW (renewal is cheap).
                if let Ok(kp) = services.identity.get_signing_keypair() {
                    let our_key = kp.public_key();
                    if let Some(handle) = registry.lookup_by_owner(&our_key) {
                        let seven_days = 7 * 24 * 3600;
                        if !handle.is_expired() && handle.expires_at <= now + seven_days {
                            let handle_name = handle.name.clone();
                            let ninety_days = 90 * 24 * 3600;
                            // Directly extend expiry (we already verified ownership)
                            if let Some(h) = registry.handles_mut().get_mut(&handle_name) {
                                h.expires_at = now + ninety_days;
                                tracing::info!(
                                    handle = %handle_name,
                                    expires_at = h.expires_at,
                                    "auto-renewed handle (was expiring within 7 days)"
                                );
                            }
                        }
                    }
                }

                tracing::debug!("handle registry GC completed");
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("handle GC loop shutting down");
                break;
            }
        }
    }
}

/// Poll the dead drop mailbox for incoming offline messages.
///
/// Runs every 5 minutes. When the identity is unlocked:
/// 1. Garbage-collects expired dead drop messages.
/// 2. Queries the local dead drop table for pending messages.
/// 3. Queries the DHT for our mailbox key to retrieve offline messages.
/// 4. Emits events for any new messages found.
pub async fn dead_drop_poll_loop(
    services: Arc<ServiceContainer>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(DEAD_DROP_POLL_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // GC expired dead drop messages regardless of identity state.
                {
                    let db = match services.metadata_db.lock() {
                        Ok(d) => d,
                        Err(_) => continue,
                    };
                    match DeadDropService::gc(&db) {
                        Ok(removed) if removed > 0 => {
                            tracing::info!(
                                removed, "dead drop GC: removed expired messages"
                            );
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(
                                error = %e, "dead drop GC failed"
                            );
                        }
                    }
                }

                // Check our mailbox if identity is unlocked.
                let pubkey = match services.identity.get_signing_keypair() {
                    Ok(kp) => kp.public_key(),
                    Err(_) => continue, // identity locked
                };

                // Query DHT for dead drop records addressed to our mailbox.
                retrieve_dht_dead_drops(&services, &pubkey);

                let db = match services.metadata_db.lock() {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                match DeadDropService::check_mailbox(&db, &pubkey) {
                    Ok(pending) if !pending.is_empty() => {
                        tracing::info!(
                            count = pending.len(),
                            "dead drop: found pending messages"
                        );
                        // Emit events and acknowledge each pending message so
                        // it is removed from the dead drop table.
                        for pm in &pending {
                            let msg_id_hex = hex::encode(pm.id.hash_bytes());
                            tracing::info!(
                                message_id = %msg_id_hex,
                                "dead drop: processing message"
                            );
                            services.event_bus.emit(Event::MessageReceived {
                                from: pubkey,
                                message_id: msg_id_hex.clone(),
                            });
                            match DeadDropService::acknowledge(&db, &pm.id) {
                                Ok(()) => {
                                    tracing::info!(
                                        message_id = %msg_id_hex,
                                        "dead drop: acknowledged message"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        message_id = %msg_id_hex,
                                        error = %e,
                                        "dead drop: failed to acknowledge message"
                                    );
                                }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(
                            error = %e, "dead drop mailbox check failed"
                        );
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("dead drop poll loop shutting down");
                break;
            }
        }
    }
}

/// Retrieve dead drop records from the DHT for the given pubkey and store
/// them in the local dead drop table. Also detects connection requests
/// and inserts them into the social connections table.
///
/// After successfully processing a DHT dead drop record (connection request,
/// connection acceptance, or regular message), the record is removed from the
/// DHT so it does not reappear on subsequent polls.
fn retrieve_dht_dead_drops(
    services: &ServiceContainer,
    pubkey: &ephemera_types::IdentityKey,
) {
    use ephemera_message::DeadDropEnvelope;

    let mailbox_key = DeadDropService::mailbox_key(pubkey);
    let dht_key = *mailbox_key.hash_bytes();

    let dht_storage = match services.dht_storage.lock() {
        Ok(s) => s,
        Err(_) => return,
    };

    let record = match dht_storage.get(&dht_key) {
        Some(r) => r,
        None => return,
    };

    // Deserialize the DHT record value as a DeadDropEnvelope.
    let envelope: DeadDropEnvelope = match serde_json::from_slice(&record.value) {
        Ok(e) => e,
        Err(_) => return,
    };

    // Drop the DHT lock before acquiring the metadata DB lock.
    drop(dht_storage);

    let db = match services.metadata_db.lock() {
        Ok(d) => d,
        Err(_) => return,
    };

    // Track whether we successfully processed this record so we can
    // remove it from the DHT afterwards.
    let mut processed = false;

    // Check if this is a connection request or acceptance (unencrypted JSON payload).
    if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&envelope.sealed_data) {
        let payload_type = payload.get("type").and_then(|t| t.as_str());

        if payload_type == Some("connection_request") {
            let initiator_hex = payload.get("initiator").and_then(|v| v.as_str()).unwrap_or("");
            let message = payload.get("message").and_then(|v| v.as_str());
            let created_at = payload.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);

            if let Ok(initiator_bytes) = hex::decode(initiator_hex) {
                if initiator_bytes.len() == 32 {
                    let local_bytes = pubkey.as_bytes().to_vec();
                    let insert_result = db.conn().execute(
                        "INSERT OR IGNORE INTO connections \
                         (local_pubkey, remote_pubkey, status, created_at, updated_at, message, initiator_pubkey) \
                         VALUES (?1, ?2, 'pending_incoming', ?3, ?3, ?4, ?2)",
                        rusqlite::params![local_bytes, initiator_bytes, created_at, message],
                    );
                    match insert_result {
                        Ok(rows) if rows > 0 => {
                            tracing::info!(
                                from = %initiator_hex,
                                "dead drop: processing message type=connection_request"
                            );
                            // Emit event for frontend notification.
                            let mut arr = [0u8; 32];
                            arr.copy_from_slice(&initiator_bytes);
                            services.event_bus.emit(Event::ConnectionRequestReceived {
                                from: ephemera_types::IdentityKey::from_bytes(arr),
                            });
                            // Store notification for notification center.
                            let _ = crate::services::NotificationService::insert(
                                &services.metadata_db,
                                "connection_request",
                                Some(&initiator_bytes),
                                None,
                                message,
                                None,
                            );
                        }
                        Ok(_) => {
                            // INSERT OR IGNORE hit a duplicate -- already processed,
                            // but the DHT record was never cleaned up. Mark as
                            // processed so we remove it now.
                            tracing::debug!(
                                from = %initiator_hex,
                                "dead drop: connection_request already exists, removing stale DHT record"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "DHT dead drop: failed to insert connection request");
                        }
                    }
                    // Whether newly inserted or a duplicate, we've handled it.
                    processed = true;
                }
            }
        } else if payload_type == Some("connection_accepted") {
            let acceptor_hex = payload.get("acceptor").and_then(|v| v.as_str()).unwrap_or("");
            let accepted_at = payload.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);

            if let Ok(acceptor_bytes) = hex::decode(acceptor_hex) {
                if acceptor_bytes.len() == 32 {
                    let local_bytes = pubkey.as_bytes().to_vec();
                    // Update our pending_outgoing to connected.
                    let update_result = db.conn().execute(
                        "UPDATE connections SET status = 'connected', updated_at = ?3 \
                         WHERE local_pubkey = ?1 AND remote_pubkey = ?2 AND status = 'pending_outgoing'",
                        rusqlite::params![local_bytes, acceptor_bytes, accepted_at],
                    );
                    match update_result {
                        Ok(rows) if rows > 0 => {
                            tracing::info!(
                                from = %acceptor_hex,
                                "dead drop: processing message type=connection_accepted"
                            );
                            let mut arr = [0u8; 32];
                            arr.copy_from_slice(&acceptor_bytes);
                            services.event_bus.emit(Event::ConnectionEstablished {
                                peer: ephemera_types::IdentityKey::from_bytes(arr),
                            });
                            // Store notification for notification center.
                            let _ = crate::services::NotificationService::insert(
                                &services.metadata_db,
                                "connection_accepted",
                                Some(&acceptor_bytes),
                                None,
                                Some("Your connection request was accepted"),
                                None,
                            );
                        }
                        Ok(_) => {
                            tracing::debug!(
                                from = %acceptor_hex,
                                "dead drop: connection_accepted but no pending_outgoing row (already processed or cancelled)"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "DHT dead drop: failed to update connection to connected");
                        }
                    }
                    // Whether we updated a row or not, we've handled this record.
                    processed = true;
                }
            }
        }
    }

    // If not a connection request/acceptance, deposit as a regular dead drop message.
    if !processed {
        let msg_id = ephemera_types::ContentId::from_digest(envelope.message_id);
        let msg_id_hex = hex::encode(envelope.message_id);
        match DeadDropService::deposit_raw(
            &db,
            &mailbox_key,
            &msg_id,
            &envelope.sealed_data,
            envelope.deposited_at,
            envelope.expires_at,
        ) {
            Ok(()) => {
                tracing::info!(
                    message_id = %msg_id_hex,
                    "dead drop: processing message type=sealed_message"
                );
                processed = true;
            }
            Err(e) => {
                // Duplicate or expired messages are expected -- still mark as
                // processed so we clean up the DHT record.
                let err_str = e.to_string();
                if err_str.contains("UNIQUE") || err_str.contains("expired") || err_str.contains("Expired") {
                    tracing::debug!(
                        message_id = %msg_id_hex,
                        "dead drop: message already stored or expired, removing stale DHT record"
                    );
                    processed = true;
                } else {
                    tracing::warn!(
                        message_id = %msg_id_hex,
                        error = %e,
                        "DHT dead drop deposit failed"
                    );
                }
            }
        }
    }

    // Drop the DB lock before re-acquiring the DHT lock.
    drop(db);

    // Remove the processed record from the DHT so it doesn't reappear.
    if processed {
        if let Ok(mut dht) = services.dht_storage.lock() {
            if dht.remove(&dht_key) {
                tracing::info!("dead drop: removed processed DHT record from mailbox");
            }
        }
    }
}

/// Apply time-based decay to all tracked reputation scores.
///
/// Runs daily. Multiplies accumulated positive and negative points by
/// a decay factor (30-day half-life), preventing permanent reputation
/// effects and incentivizing continued participation.
pub async fn reputation_decay_loop(
    services: Arc<ServiceContainer>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(REPUTATION_DECAY_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let mut rep_map = match services.reputation.lock() {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                let count = rep_map.len();
                for score in rep_map.values_mut() {
                    score.apply_decay(1.0); // 1 day of decay
                }

                if count > 0 {
                    tracing::debug!(
                        identities = count,
                        "reputation decay applied"
                    );
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("reputation decay loop shutting down");
                break;
            }
        }
    }
}

/// Periodically refresh connected users' profiles from the DHT.
///
/// Runs every 30 minutes. For each active connection, looks up the remote
/// peer's profile in the DHT and updates the local profiles table if a
/// newer version is found.
pub async fn profile_refresh_loop(
    services: Arc<ServiceContainer>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    use crate::services::dht::DhtNodeService;

    let mut interval = tokio::time::interval(PROFILE_REFRESH_INTERVAL);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Get the local identity (skip if locked).
                let local = match services.identity.get_signing_keypair() {
                    Ok(kp) => kp.public_key(),
                    Err(_) => continue,
                };

                // List all active connections.
                let connections: Vec<ephemera_social::Connection> = match services.social.social_services
                    .list(&local, Some(ephemera_social::ConnectionStatus::Active))
                    .await
                {
                    Ok(conns) => conns,
                    Err(e) => {
                        tracing::debug!(error = %e, "profile refresh: failed to list connections");
                        continue;
                    }
                };

                if connections.is_empty() {
                    continue;
                }

                let mut refreshed = 0u32;
                for conn in &connections {
                    // Determine the remote peer's pubkey.
                    let remote = if conn.initiator == local {
                        &conn.responder
                    } else {
                        &conn.initiator
                    };
                    let remote_hex = hex::encode(remote.as_bytes());

                    // Look up profile from DHT.
                    let profile = match DhtNodeService::lookup_profile(&remote_hex, &services.dht_storage) {
                        Ok(Some(p)) => p,
                        _ => continue,
                    };

                    let display_name = profile.get("display_name").and_then(|v| v.as_str());
                    let bio = profile.get("bio").and_then(|v| v.as_str());

                    if display_name.is_none() && bio.is_none() {
                        continue;
                    }

                    // Store in local profiles table.
                    if let Ok(db) = services.metadata_db.lock() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        let empty_sig: Vec<u8> = vec![0u8; 64];
                        let pk_bytes = remote.as_bytes().to_vec();
                        let _ = db.conn().execute(
                            "INSERT INTO profiles (pubkey, display_name, bio, updated_at, signature, received_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?4)
                             ON CONFLICT(pubkey) DO UPDATE SET
                                display_name = COALESCE(?2, display_name),
                                bio = COALESCE(?3, bio),
                                updated_at = ?4, signature = ?5, received_at = ?4",
                            rusqlite::params![pk_bytes, display_name, bio, now, empty_sig],
                        );
                        refreshed += 1;
                    }
                }

                if refreshed > 0 {
                    tracing::info!(
                        refreshed,
                        total = connections.len(),
                        "profile refresh: updated connected user profiles from DHT"
                    );
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("profile refresh loop shutting down");
                break;
            }
        }
    }
}
