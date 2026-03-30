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
                        // Emit events for each pending message so the frontend
                        // can display a notification badge.
                        for pm in &pending {
                            let msg_id_hex = hex::encode(pm.id.hash_bytes());
                            services.event_bus.emit(Event::MessageReceived {
                                from: pubkey,
                                message_id: msg_id_hex,
                            });
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

    if let Some(record) = dht_storage.get(&dht_key) {
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
                                    "DHT dead drop: received connection request"
                                );
                                // Emit event for frontend notification.
                                let mut arr = [0u8; 32];
                                arr.copy_from_slice(&initiator_bytes);
                                services.event_bus.emit(Event::ConnectionRequestReceived {
                                    from: ephemera_types::IdentityKey::from_bytes(arr),
                                });
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(error = %e, "DHT dead drop: failed to insert connection request");
                            }
                        }
                        // Don't store connection requests in the dead drop table.
                        return;
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
                                    "DHT dead drop: connection accepted, updated to connected"
                                );
                                let mut arr = [0u8; 32];
                                arr.copy_from_slice(&acceptor_bytes);
                                services.event_bus.emit(Event::ConnectionEstablished {
                                    peer: ephemera_types::IdentityKey::from_bytes(arr),
                                });
                            }
                            Ok(_) => {
                                tracing::debug!(
                                    from = %acceptor_hex,
                                    "DHT dead drop: connection acceptance but no pending_outgoing row found"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "DHT dead drop: failed to update connection to connected");
                            }
                        }
                        // Don't store acceptances in the dead drop table.
                        return;
                    }
                }
            }
        }

        let msg_id = ephemera_types::ContentId::from_digest(envelope.message_id);
        if let Err(e) = DeadDropService::deposit_raw(
            &db,
            &mailbox_key,
            &msg_id,
            &envelope.sealed_data,
            envelope.deposited_at,
            envelope.expires_at,
        ) {
            tracing::trace!(
                error = %e,
                "DHT dead drop deposit failed (likely dup or expired)"
            );
        } else {
            tracing::debug!("dead drop poll: ingested message from DHT");
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
