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

                // Drop the DB lock before emitting events.
                drop(db);

                // If addressed to us, emit a MessageReceived event.
                if is_for_us {
                    if let Some(ref pk) = our_pubkey {
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
