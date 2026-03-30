//! Message ingest: receives dead drop envelopes from the gossip network,
//! validates them, and stores them in the local dead drop table.
//!
//! This module is the DM counterpart to `gossip_ingest.rs` (which handles
//! public posts). When a dead drop envelope arrives on the `dm_delivery`
//! gossip topic:
//! 1. Deserialize the envelope from the payload.
//! 2. Validate expiry and size constraints.
//! 3. Check if the mailbox key matches our pubkey (we are the recipient).
//! 4. Store in the local dead drop table for later retrieval.
//! 5. If we are the recipient, emit a MessageReceived event.

use ephemera_events::{Event, EventBus};
use ephemera_gossip::TopicSubscription;
use ephemera_message::dead_drop::DEAD_DROP_MAX_TTL_SECS;
use ephemera_message::{DeadDropEnvelope, DeadDropService};
use ephemera_store::MetadataDb;
use ephemera_types::{ContentId, IdentityKey};

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

/// Maximum size of a serialized dead drop envelope on the wire (64 KiB).
const MAX_ENVELOPE_SIZE: usize = 64 * 1024;

/// Process incoming dead drop envelopes from the `dm_delivery` gossip topic.
///
/// This function runs as a background task. It reads messages from the gossip
/// subscription, validates them as dead drop envelopes, and stores valid ones
/// in the local dead drop table.
///
/// If the envelope is addressed to our mailbox, a `MessageReceived` event is
/// emitted so the frontend can notify the user.
///
/// Exits when the subscription channel closes or the shutdown signal fires.
pub async fn message_ingest_loop(
    mut subscription: TopicSubscription,
    metadata_db: Mutex<MetadataDb>,
    event_bus: EventBus,
    our_pubkey: Option<IdentityKey>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    // Pre-compute our mailbox key if identity is available.
    let our_mailbox = our_pubkey.map(|pk| DeadDropService::mailbox_key(&pk));

    loop {
        tokio::select! {
            msg = subscription.recv() => {
                let msg = match msg {
                    Some(m) => m,
                    None => {
                        tracing::debug!("message ingest: subscription channel closed");
                        break;
                    }
                };

                // Size check before deserialization.
                if msg.payload.len() > MAX_ENVELOPE_SIZE {
                    tracing::warn!(
                        size = msg.payload.len(),
                        "message ingest: envelope too large, dropping"
                    );
                    continue;
                }

                // Deserialize the gossip payload as a DeadDropEnvelope.
                let envelope: DeadDropEnvelope = match serde_json::from_slice(&msg.payload) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::trace!(
                            error = %e,
                            "message ingest: payload is not a valid DeadDropEnvelope"
                        );
                        continue;
                    }
                };

                // Validate expiry.
                let now = now_secs();
                if envelope.expires_at <= now {
                    tracing::trace!("message ingest: expired envelope, dropping");
                    continue;
                }

                // Clamp expires_at to prevent far-future abuse.
                let clamped_expires = envelope
                    .expires_at
                    .min(now + DEAD_DROP_MAX_TTL_SECS);

                let mailbox_key = ContentId::from_digest(envelope.mailbox_key);
                let message_id = ContentId::from_digest(envelope.message_id);

                // Store in local dead drop table (relay for the network).
                let is_for_us = our_mailbox
                    .as_ref()
                    .is_some_and(|ours| *ours == mailbox_key);

                let db = match metadata_db.lock() {
                    Ok(d) => d,
                    Err(_) => {
                        tracing::warn!("message ingest: failed to lock metadata db");
                        continue;
                    }
                };

                match DeadDropService::deposit_raw(
                    &db,
                    &mailbox_key,
                    &message_id,
                    &envelope.sealed_data,
                    envelope.deposited_at,
                    clamped_expires,
                ) {
                    Ok(()) => {
                        tracing::debug!(
                            message_id = %message_id,
                            for_us = is_for_us,
                            "message ingest: stored dead drop from network"
                        );
                    }
                    Err(e) => {
                        // deposit_raw returns Err for duplicates (INSERT OR IGNORE
                        // succeeds silently) or truly expired records. Both are fine.
                        tracing::trace!(
                            error = %e,
                            "message ingest: deposit_raw returned error (likely dup or expired)"
                        );
                        continue;
                    }
                }

                // If addressed to us, check if it's a connection request or acceptance.
                // These payloads are JSON with type: "connection_request" or
                // "connection_accepted" (NOT encrypted, unlike DM messages).
                let mut is_connection_request = false;
                let mut is_connection_accepted = false;
                if is_for_us {
                    if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&envelope.sealed_data) {
                        let payload_type = payload.get("type").and_then(|t| t.as_str());

                        if payload_type == Some("connection_request") {
                            is_connection_request = true;
                            // Extract initiator and responder pubkeys.
                            let initiator_hex = payload.get("initiator").and_then(|v| v.as_str()).unwrap_or("");
                            let message = payload.get("message").and_then(|v| v.as_str());
                            let created_at = payload.get("created_at").and_then(|v| v.as_i64()).unwrap_or(now as i64);

                            if let (Ok(initiator_bytes), Some(ref pk)) = (hex::decode(initiator_hex), &our_pubkey) {
                                if initiator_bytes.len() == 32 {
                                    let local_bytes = pk.as_bytes().to_vec();
                                    // Insert as pending_incoming connection (ignore if already exists).
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
                                                "message ingest: received connection request"
                                            );
                                        }
                                        Ok(_) => {
                                            tracing::debug!(
                                                from = %initiator_hex,
                                                "message ingest: connection request already exists (ignored)"
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "message ingest: failed to insert connection request"
                                            );
                                        }
                                    }
                                }
                            }
                        } else if payload_type == Some("connection_accepted") {
                            is_connection_accepted = true;
                            let acceptor_hex = payload.get("acceptor").and_then(|v| v.as_str()).unwrap_or("");
                            let accepted_at = payload.get("created_at").and_then(|v| v.as_i64()).unwrap_or(now as i64);

                            if let (Ok(acceptor_bytes), Some(ref pk)) = (hex::decode(acceptor_hex), &our_pubkey) {
                                if acceptor_bytes.len() == 32 {
                                    let local_bytes = pk.as_bytes().to_vec();
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
                                                "message ingest: connection accepted, updated to connected"
                                            );
                                        }
                                        Ok(_) => {
                                            tracing::debug!(
                                                from = %acceptor_hex,
                                                "message ingest: connection acceptance received but no pending_outgoing row found"
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "message ingest: failed to update connection to connected"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Drop the DB lock before emitting events.
                drop(db);

                // If addressed to us, emit the appropriate event:
                // - connection_request -> ConnectionRequestReceived
                // - connection_accepted -> ConnectionEstablished
                // - regular message -> MessageReceived
                if is_for_us {
                    if is_connection_request {
                        if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&envelope.sealed_data) {
                            let initiator_hex = payload.get("initiator").and_then(|v| v.as_str()).unwrap_or("");
                            if let Ok(init_bytes) = hex::decode(initiator_hex) {
                                if init_bytes.len() == 32 {
                                    let mut arr = [0u8; 32];
                                    arr.copy_from_slice(&init_bytes);
                                    let from_key = IdentityKey::from_bytes(arr);
                                    event_bus.emit(Event::ConnectionRequestReceived {
                                        from: from_key,
                                    });
                                }
                            }
                        }
                    } else if is_connection_accepted {
                        if let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&envelope.sealed_data) {
                            let acceptor_hex = payload.get("acceptor").and_then(|v| v.as_str()).unwrap_or("");
                            if let Ok(acc_bytes) = hex::decode(acceptor_hex) {
                                if acc_bytes.len() == 32 {
                                    let mut arr = [0u8; 32];
                                    arr.copy_from_slice(&acc_bytes);
                                    let peer_key = IdentityKey::from_bytes(arr);
                                    event_bus.emit(Event::ConnectionEstablished {
                                        peer: peer_key,
                                    });
                                }
                            }
                        }
                    } else if let Some(ref pk) = our_pubkey {
                        let msg_id_hex = hex::encode(envelope.message_id);
                        tracing::info!(
                            message_id = %msg_id_hex,
                            "message ingest: received message addressed to us"
                        );
                        event_bus.emit(Event::MessageReceived {
                            from: *pk, // sender is unknown (sealed), use our key as placeholder
                            message_id: msg_id_hex,
                        });
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::debug!("message ingest: received shutdown signal");
                break;
            }
        }
    }
}
