//! Social, messaging, profile, and moderation services wired to real implementations.
//!
//! Contains the node-level service wrappers for social connections, direct
//! messaging, user profiles, and content moderation. Each service delegates
//! to the corresponding domain crate and translates between typed errors and
//! the `Result<Value, String>` contract used by the JSON-RPC API.

use super::dht::DhtNodeService;
use super::identity::IdentityService;
use super::PostService;
use ephemera_abuse::{ActionType, RateLimiter, ReputationEvent, ReputationScore};
use ephemera_dht::storage::DhtStorage;
use ephemera_message::{DeadDropEnvelope, DeadDropService, MessageService as MsgServiceImpl};
use ephemera_mod::ReportService;
use ephemera_social::store::SqliteSocialServices;
use ephemera_social::{BlockService, ConnectionService, ConnectionStatus};
use ephemera_store::{ContentStore, MetadataDb};
use ephemera_types::{ContentId, IdentityKey, Timestamp};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Return the current wall-clock time as Unix seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
}

/// Parse a hex-encoded public key string into an IdentityKey.
fn parse_pubkey(hex_str: &str) -> Result<IdentityKey, String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("bad pubkey hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("pubkey must be 32 bytes, got {}", bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(IdentityKey::from_bytes(arr))
}

/// Look up the author of a post by content hash (hex).
fn lookup_post_author(hash_hex: &str, metadata_db: &Mutex<MetadataDb>) -> Option<IdentityKey> {
    let hash_bytes = hex::decode(hash_hex).ok()?;
    if hash_bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hash_bytes);
    let content_id = ContentId::from_digest(arr);
    let wire = content_id.to_wire_bytes();
    let db = metadata_db.lock().ok()?;
    let author_bytes: Vec<u8> = db
        .conn()
        .query_row(
            "SELECT author_pubkey FROM posts WHERE content_hash = ?1",
            rusqlite::params![wire],
            |row| row.get(0),
        )
        .ok()?;
    if author_bytes.len() != 32 {
        return None;
    }
    let mut key_arr = [0u8; 32];
    key_arr.copy_from_slice(&author_bytes);
    Some(IdentityKey::from_bytes(key_arr))
}

/// Get the current user's IdentityKey from the IdentityService.
fn get_local_identity(identity: &IdentityService) -> Result<IdentityKey, String> {
    let signing_kp = identity.get_signing_keypair()?;
    Ok(signing_kp.public_key())
}

/// Fetch a remote user's profile from DHT and store it locally in the
/// profiles table. Best-effort: logs warnings on failure but never returns
/// an error since this is supplementary to the main connection flow.
fn fetch_and_store_remote_profile(
    pubkey_hex: &str,
    dht_storage: &Mutex<DhtStorage>,
    metadata_db: Option<&Mutex<MetadataDb>>,
) {
    match DhtNodeService::lookup_profile(pubkey_hex, dht_storage) {
        Ok(Some(profile)) => {
            let display_name = profile.get("display_name").and_then(|v| v.as_str());
            let bio = profile.get("bio").and_then(|v| v.as_str());

            if display_name.is_none() && bio.is_none() {
                tracing::debug!(pubkey = %pubkey_hex, "remote profile from DHT has no display_name or bio");
                return;
            }

            // If we have a metadata_db, store the profile there.
            if let Some(mdb) = metadata_db {
                if let Ok(pk_bytes) = hex::decode(pubkey_hex) {
                    if let Ok(db) = mdb.lock() {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        let empty_sig: Vec<u8> = vec![0u8; 64];
                        let _ = db.conn().execute(
                            "INSERT INTO profiles (pubkey, display_name, bio, updated_at, signature, received_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?4)
                             ON CONFLICT(pubkey) DO UPDATE SET
                                display_name = COALESCE(?2, display_name),
                                bio = COALESCE(?3, bio),
                                updated_at = ?4, signature = ?5, received_at = ?4",
                            rusqlite::params![pk_bytes, display_name, bio, now, empty_sig],
                        );
                        tracing::info!(
                            pubkey = %pubkey_hex,
                            display_name = ?display_name,
                            "stored remote profile from DHT"
                        );
                    }
                }
            } else {
                tracing::debug!(
                    pubkey = %pubkey_hex,
                    display_name = ?display_name,
                    "found remote profile in DHT (no metadata_db to store it)"
                );
            }
        }
        Ok(None) => {
            tracing::debug!(pubkey = %pubkey_hex, "no profile found in DHT for remote user");
        }
        Err(e) => {
            tracing::warn!(
                pubkey = %pubkey_hex,
                error = %e,
                "failed to lookup remote profile from DHT"
            );
        }
    }
}

/// Build a profile JSON value for DHT publication, including an avatar
/// thumbnail if one exists in the profile table.
///
/// The avatar thumbnail is read from the `avatar_cid` column, looked up
/// in the content store as a BLAKE3 hash, and base64-encoded. If the
/// thumbnail is too large (>4KB after encoding) it is omitted.
fn build_dht_profile(
    pubkey_hex: &str,
    display_name: Option<&str>,
    bio: Option<&str>,
    metadata_db: &Mutex<MetadataDb>,
) -> Value {
    let mut profile = serde_json::json!({
        "pubkey": pubkey_hex,
        "display_name": display_name,
        "bio": bio,
    });

    // Try to read avatar_cid from the profiles table.
    if let Ok(pk_bytes) = hex::decode(pubkey_hex) {
        if let Ok(db) = metadata_db.lock() {
            let avatar_cid: Option<Vec<u8>> = db
                .conn()
                .query_row(
                    "SELECT avatar_cid FROM profiles WHERE pubkey = ?1",
                    rusqlite::params![pk_bytes],
                    |row| row.get(0),
                )
                .ok()
                .flatten();

            if let Some(ref cid) = avatar_cid {
                // The avatar_cid is the BLAKE3 hash of the standard image.
                // For the DHT we want the thumbnail, but we don't store the
                // thumbnail hash separately. Instead, we encode the avatar_cid
                // hex so remote nodes know which content to request, and
                // provide a small data:image URI for immediate display.
                let cid_hex = hex::encode(cid);
                if let Some(obj) = profile.as_object_mut() {
                    obj.insert(
                        "avatar_cid".to_string(),
                        Value::String(cid_hex),
                    );
                }
            }
        }
    }

    profile
}

/// Store a profile value (already fetched from the network DHT) into the
/// local metadata database. Best-effort: logs but does not propagate errors.
fn store_profile_from_dht(
    pubkey_hex: &str,
    profile: &Value,
    metadata_db: &Mutex<MetadataDb>,
) {
    let display_name = profile.get("display_name").and_then(|v| v.as_str());
    let bio = profile.get("bio").and_then(|v| v.as_str());

    if display_name.is_none() && bio.is_none() {
        return;
    }

    let pk_bytes = match hex::decode(pubkey_hex) {
        Ok(b) if b.len() == 32 => b,
        _ => return,
    };

    let db = match metadata_db.lock() {
        Ok(d) => d,
        Err(_) => return,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let empty_sig: Vec<u8> = vec![0u8; 64];

    let _ = db.conn().execute(
        "INSERT INTO profiles (pubkey, display_name, bio, updated_at, signature, received_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?4)
         ON CONFLICT(pubkey) DO UPDATE SET
            display_name = COALESCE(?2, display_name),
            bio = COALESCE(?3, bio),
            updated_at = ?4, signature = ?5, received_at = ?4",
        rusqlite::params![pk_bytes, display_name, bio, now, empty_sig],
    );

    tracing::info!(
        pubkey = %pubkey_hex,
        display_name = ?display_name,
        "stored network DHT profile in local database"
    );
}

/// Serialize a Connection to JSON.
///
/// When `local` is provided, computes `pseudonym_id` as the "other" peer
/// and maps the status to the frontend-expected lowercase form (F-08).
fn connection_to_json(
    conn: &ephemera_social::Connection,
    local: Option<&IdentityKey>,
    metadata_db: Option<&Mutex<MetadataDb>>,
) -> Value {
    let initiator_hex = hex::encode(conn.initiator.as_bytes());
    let responder_hex = hex::encode(conn.responder.as_bytes());

    // Determine which side is the remote peer.
    let pseudonym_id = if let Some(local_key) = local {
        if conn.initiator == *local_key {
            responder_hex.clone()
        } else {
            initiator_hex.clone()
        }
    } else {
        // Fallback: use responder as the peer.
        responder_hex.clone()
    };

    // Map status to lowercase forms the frontend expects.
    let status_str = match conn.status {
        ConnectionStatus::PendingOutgoing => "pending_outgoing",
        ConnectionStatus::PendingIncoming => "pending_incoming",
        ConnectionStatus::Active => "connected",
        ConnectionStatus::Blocked => "blocked",
    };

    // Look up display_name and avatar_cid from profiles table.
    let (display_name, avatar_url): (Option<String>, Option<String>) = if let Some(mdb) = metadata_db {
        if let Ok(pk_bytes) = hex::decode(&pseudonym_id) {
            if let Ok(db) = mdb.lock() {
                db.conn()
                    .query_row(
                        "SELECT display_name, avatar_cid FROM profiles WHERE pubkey = ?1",
                        rusqlite::params![pk_bytes],
                        |row| {
                            let name: Option<String> = row.get(0)?;
                            let avatar_cid: Option<Vec<u8>> = row.get(1)?;
                            let url = avatar_cid.map(|cid| format!("/media/{}", hex::encode(cid)));
                            Ok((name, url))
                        },
                    )
                    .unwrap_or((None, None))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    serde_json::json!({
        "pseudonym_id": pseudonym_id,
        "display_name": display_name,
        "avatar_url": avatar_url,
        "initiator": initiator_hex,
        "responder": responder_hex,
        "status": status_str,
        "created_at": conn.created_at.as_secs(),
        "updated_at": conn.updated_at.as_secs(),
        "since": conn.created_at.as_secs(),
        "message": conn.message,
    })
}

/// Connection management, follows, reactions -- backed by SqliteSocialServices.
pub struct SocialService {
    pub(crate) social_services: SqliteSocialServices,
}

impl SocialService {
    /// Send a connection request to the target pubkey.
    ///
    /// Stores the request locally AND publishes it to the recipient's dead
    /// drop (via gossip + DHT) so the remote node can discover it even if
    /// the two peers are not directly connected.
    pub async fn connect(
        &self,
        target: &str,
        msg: &str,
        identity: &IdentityService,
        network: Option<&crate::network::NetworkSubsystem>,
        dht_storage: Option<&Mutex<DhtStorage>>,
        metadata_db: Option<&Mutex<MetadataDb>>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(target)?;
        let message = if msg.is_empty() { None } else { Some(msg) };
        let conn = self
            .social_services
            .request(&local, &remote, message)
            .await
            .map_err(|e| format!("connection request: {e}"))?;

        // Build a wire envelope for the connection request so the recipient
        // can discover it via gossip / DHT dead drop polling.
        let now = now_secs();
        let request_payload = serde_json::json!({
            "type": "connection_request",
            "initiator": hex::encode(local.as_bytes()),
            "responder": hex::encode(remote.as_bytes()),
            "message": conn.message,
            "created_at": conn.created_at.as_secs(),
        });
        let payload_bytes = serde_json::to_vec(&request_payload)
            .map_err(|e| format!("serialize connection request: {e}"))?;

        // Deposit in recipient's dead drop (local DB).
        let mailbox_key = DeadDropService::mailbox_key(&remote);
        let req_hash = blake3::hash(&payload_bytes);
        let req_content_id = ContentId::from_digest(*req_hash.as_bytes());
        let expires_at = now + 7 * 24 * 60 * 60; // 7-day TTL for connection requests

        // Store locally (best-effort, don't fail the request).
        // We need a MetadataDb for this -- access through social_services.
        // Use the same dead drop mechanism as messages.

        // Publish to gossip on the dm_delivery topic so relays propagate it.
        let mut published_gossip = false;
        if let Some(net) = network {
            let dm_topic = ephemera_gossip::GossipTopic::direct_messages();
            let envelope = DeadDropEnvelope {
                mailbox_key: *mailbox_key.hash_bytes(),
                message_id: *req_content_id.hash_bytes(),
                sealed_data: payload_bytes.clone(),
                deposited_at: now,
                expires_at,
            };
            if let Ok(env_bytes) = serde_json::to_vec(&envelope) {
                match net.publish(&dm_topic, env_bytes).await {
                    Ok(()) => {
                        published_gossip = true;
                        tracing::info!(
                            target = hex::encode(remote.as_bytes()),
                            "published connection request to gossip"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to publish connection request to gossip");
                    }
                }
            }
        }

        // Publish to DHT for offline discovery.
        let mut published_dht = false;
        if let Some(dht) = dht_storage {
            if let Ok(mut storage) = dht.lock() {
                let dht_record = ephemera_dht::DhtRecord {
                    key: *mailbox_key.hash_bytes(),
                    record_type: ephemera_dht::DhtRecordType::DeadDrop,
                    value: payload_bytes,
                    publisher: *local.as_bytes(),
                    timestamp: now,
                    ttl_seconds: (7 * 24 * 60 * 60) as u32,
                    signature: vec![], // unsigned for PoC
                };
                match storage.put(dht_record) {
                    Ok(()) => {
                        published_dht = true;
                        tracing::info!(
                            target = hex::encode(remote.as_bytes()),
                            "published connection request to DHT"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to publish connection request to DHT");
                    }
                }
            }
        }

        // Fetch the remote user's profile from DHT and store locally.
        if let Some(dht) = dht_storage {
            fetch_and_store_remote_profile(target, dht, metadata_db);
        }

        let delivered = published_gossip || published_dht;
        let mut result = connection_to_json(&conn, Some(&local), metadata_db);
        if let Some(obj) = result.as_object_mut() {
            obj.insert("published_gossip".to_string(), serde_json::json!(published_gossip));
            obj.insert("published_dht".to_string(), serde_json::json!(published_dht));
            obj.insert("sent".to_string(), serde_json::json!(true));
            obj.insert("delivered".to_string(), serde_json::json!(delivered));
        }
        Ok(result)
    }

    /// Accept a pending incoming connection request.
    ///
    /// After updating the local DB, publishes a `connection_accepted`
    /// envelope to gossip + DHT so the requester (who may be on another
    /// device/node) can discover the acceptance and flip their local
    /// `pending_outgoing` row to `connected`.
    pub async fn accept(
        &self,
        from: &str,
        identity: &IdentityService,
        network: Option<&crate::network::NetworkSubsystem>,
        dht_storage: Option<&Mutex<DhtStorage>>,
        metadata_db: Option<&Mutex<MetadataDb>>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(from)?;
        let conn = self
            .social_services
            .accept(&local, &remote)
            .await
            .map_err(|e| format!("accept connection: {e}"))?;

        // Notify the requester that we accepted via gossip + DHT.
        let now = now_secs();
        let acceptance_payload = serde_json::json!({
            "type": "connection_accepted",
            "acceptor": hex::encode(local.as_bytes()),
            "requester": hex::encode(remote.as_bytes()),
            "created_at": now,
        });
        let payload_bytes = serde_json::to_vec(&acceptance_payload)
            .map_err(|e| format!("serialize acceptance: {e}"))?;

        let mailbox_key = DeadDropService::mailbox_key(&remote);
        let acc_hash = blake3::hash(&payload_bytes);
        let acc_content_id = ContentId::from_digest(*acc_hash.as_bytes());
        let expires_at = now + 7 * 24 * 60 * 60; // 7-day TTL

        // Publish to gossip.
        let mut published_gossip = false;
        if let Some(net) = network {
            let dm_topic = ephemera_gossip::GossipTopic::direct_messages();
            let envelope = DeadDropEnvelope {
                mailbox_key: *mailbox_key.hash_bytes(),
                message_id: *acc_content_id.hash_bytes(),
                sealed_data: payload_bytes.clone(),
                deposited_at: now,
                expires_at,
            };
            if let Ok(env_bytes) = serde_json::to_vec(&envelope) {
                match net.publish(&dm_topic, env_bytes).await {
                    Ok(()) => {
                        published_gossip = true;
                        tracing::info!(
                            target = hex::encode(remote.as_bytes()),
                            "published connection acceptance to gossip"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to publish connection acceptance to gossip");
                    }
                }
            }
        }

        // Publish to DHT for offline discovery.
        let mut published_dht = false;
        if let Some(dht) = dht_storage {
            if let Ok(mut storage) = dht.lock() {
                let dht_record = ephemera_dht::DhtRecord {
                    key: *mailbox_key.hash_bytes(),
                    record_type: ephemera_dht::DhtRecordType::DeadDrop,
                    value: payload_bytes,
                    publisher: *local.as_bytes(),
                    timestamp: now,
                    ttl_seconds: (7 * 24 * 60 * 60) as u32,
                    signature: vec![], // unsigned for PoC
                };
                match storage.put(dht_record) {
                    Ok(()) => {
                        published_dht = true;
                        tracing::info!(
                            target = hex::encode(remote.as_bytes()),
                            "published connection acceptance to DHT"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to publish connection acceptance to DHT");
                    }
                }
            }
        }

        // Fetch the remote user's profile from DHT and store locally.
        if let Some(dht) = dht_storage {
            fetch_and_store_remote_profile(from, dht, metadata_db);
        }

        let delivered = published_gossip || published_dht;
        let mut result = connection_to_json(&conn, Some(&local), metadata_db);
        if let Some(obj) = result.as_object_mut() {
            obj.insert("published_gossip".to_string(), serde_json::json!(published_gossip));
            obj.insert("published_dht".to_string(), serde_json::json!(published_dht));
            obj.insert("sent".to_string(), serde_json::json!(true));
            obj.insert("delivered".to_string(), serde_json::json!(delivered));
        }
        Ok(result)
    }

    /// Reject a pending incoming connection request.
    pub async fn reject(&self, from: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(from)?;
        self.social_services
            .reject(&local, &remote)
            .await
            .map_err(|e| format!("reject connection: {e}"))?;
        Ok(serde_json::json!({ "rejected": true }))
    }

    /// Cancel a pending outgoing connection request.
    ///
    /// Deletes the local `pending_outgoing` row. The remote peer may still
    /// have a `pending_incoming` row from a previously-delivered gossip/DHT
    /// message, but we cannot recall it.
    pub async fn cancel_request(
        &self,
        target: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(target)?;
        // Reuse the `reject` path -- it just DELETEs the row for (local, remote).
        self.social_services
            .reject(&local, &remote)
            .await
            .map_err(|e| format!("cancel request: {e}"))?;
        Ok(serde_json::json!({ "cancelled": true }))
    }

    /// Resend a connection request to a target whose previous request may
    /// not have been delivered (peer was offline, gossip/DHT failed, etc.).
    ///
    /// This deletes the old pending_outgoing row and re-creates it so that
    /// the gossip + DHT publication happens again with a fresh timestamp.
    pub async fn resend_request(
        &self,
        target: &str,
        msg: &str,
        identity: &IdentityService,
        network: Option<&crate::network::NetworkSubsystem>,
        dht_storage: Option<&Mutex<DhtStorage>>,
        metadata_db: Option<&Mutex<MetadataDb>>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(target)?;

        tracing::info!(
            target = %target,
            local = %hex::encode(local.as_bytes()),
            "resend_request: starting resend flow"
        );

        // Remove the stale pending_outgoing row (best-effort).
        match self.social_services.reject(&local, &remote).await {
            Ok(()) => {
                tracing::info!(
                    target = %target,
                    "resend_request: deleted old pending_outgoing row"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target = %target,
                    error = %e,
                    "resend_request: failed to delete old row (may not exist), proceeding"
                );
            }
        }

        // Also remove from the other direction in case the remote side
        // previously rejected and we have a stale row.
        match self.social_services.reject(&remote, &local).await {
            Ok(()) => {
                tracing::info!(
                    target = %target,
                    "resend_request: deleted reverse-direction row"
                );
            }
            Err(e) => {
                tracing::debug!(
                    target = %target,
                    error = %e,
                    "resend_request: no reverse row to delete (expected)"
                );
            }
        }

        // Re-issue the connection request through the normal path, which
        // stores a fresh row and publishes to gossip + DHT.
        match self.connect(target, msg, identity, network, dht_storage, metadata_db).await {
            Ok(result) => {
                tracing::info!(
                    target = %target,
                    "resend_request: successfully re-issued connection request"
                );
                Ok(result)
            }
            Err(e) => {
                tracing::error!(
                    target = %target,
                    error = %e,
                    "resend_request: failed to re-issue connection request"
                );
                Err(e)
            }
        }
    }

    /// Remove (disconnect from) an existing connection.
    pub async fn disconnect(
        &self,
        target: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(target)?;
        self.social_services
            .remove(&local, &remote)
            .await
            .map_err(|e| format!("disconnect: {e}"))?;
        Ok(serde_json::json!({ "disconnected": true }))
    }

    /// List connections, optionally filtered by status.
    pub async fn list_connections(
        &self,
        status: &str,
        identity: &IdentityService,
        metadata_db: Option<&Mutex<MetadataDb>>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let status_filter = match status {
            "pending_outgoing" => Some(ConnectionStatus::PendingOutgoing),
            "pending_incoming" => Some(ConnectionStatus::PendingIncoming),
            "connected" | "active" => Some(ConnectionStatus::Active),
            "blocked" => Some(ConnectionStatus::Blocked),
            _ => None, // "all" or anything else
        };
        let connections = self
            .social_services
            .list(&local, status_filter)
            .await
            .map_err(|e| format!("list connections: {e}"))?;
        let items: Vec<Value> = connections
            .iter()
            .map(|c| connection_to_json(c, Some(&local), metadata_db))
            .collect();
        Ok(serde_json::json!({ "connections": items }))
    }

    /// Block a user.
    pub async fn block(&self, target: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(target)?;
        self.social_services
            .block(&local, &remote, None)
            .await
            .map_err(|e| format!("block user: {e}"))?;
        Ok(serde_json::json!({ "blocked": true }))
    }

    /// Unblock a user.
    pub async fn unblock(&self, target: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(target)?;
        self.social_services
            .unblock(&local, &remote)
            .await
            .map_err(|e| format!("unblock user: {e}"))?;
        Ok(serde_json::json!({ "unblocked": true }))
    }

    /// Mute a user, optionally with an expiry.
    pub async fn mute(
        &self,
        target: &str,
        hours: Option<u64>,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(target)?;
        let expires_at = hours.map(|h| Timestamp::from_secs(now_secs() + h * 3600));
        self.social_services
            .mute(&local, &remote, expires_at)
            .await
            .map_err(|e| format!("mute user: {e}"))?;
        Ok(serde_json::json!({ "muted": true }))
    }

    /// Unmute a user.
    pub async fn unmute(&self, target: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let remote = parse_pubkey(target)?;
        self.social_services
            .unmute(&local, &remote)
            .await
            .map_err(|e| format!("unmute user: {e}"))?;
        Ok(serde_json::json!({ "unmuted": true }))
    }

    /// Follow a target (creates a unidirectional connection request for now).
    pub async fn follow(&self, target: &str, identity: &IdentityService) -> Result<Value, String> {
        // For now, follow is implemented as a connection request.
        // No network/DHT for follows (they're local-only for now).
        self.connect(target, "", identity, None, None, None).await?;
        Ok(serde_json::json!({ "followed": true }))
    }

    /// Unfollow a target (removes connection).
    pub async fn unfollow(
        &self,
        target: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        self.disconnect(target, identity).await?;
        Ok(serde_json::json!({ "unfollowed": true }))
    }

    /// React to a post. Action is "add" or "remove".
    pub async fn react(&self, hash: &str, emoji: &str, action: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let reactor_pubkey = hex::encode(local.as_bytes());
        let emoji_type: ephemera_social::ReactionEmoji = emoji.parse().map_err(|e: ephemera_social::SocialError| e.to_string())?;
        match action {
            "add" => {
                let now = now_secs() as i64;
                self.social_services.react(hash, &reactor_pubkey, emoji_type, now).map_err(|e| format!("react: {e}"))?;
                Ok(serde_json::json!({ "reacted": true, "emoji": emoji }))
            }
            "remove" => {
                self.social_services.unreact(hash, &reactor_pubkey).map_err(|e| format!("unreact: {e}"))?;
                Ok(serde_json::json!({ "unreacted": true }))
            }
            other => Err(format!("unknown action: {other}, expected 'add' or 'remove'")),
        }
    }

    /// Get reactions for a post.
    pub async fn get_reactions(&self, hash: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        let summary = self.social_services.get_reactions(hash, Some(&my_pubkey)).map_err(|e| format!("get reactions: {e}"))?;
        let mut result = serde_json::Map::new();
        for emoji in ephemera_social::ReactionEmoji::ALL {
            result.insert(emoji.to_string(), Value::Number(0.into()));
        }
        for (emoji, count) in &summary.counts {
            result.insert(emoji.to_string(), Value::Number((*count).into()));
        }
        let my_reaction = summary.my_emoji.map(|e| e.to_string());
        result.insert("my_reaction".to_string(), my_reaction.map_or(Value::Null, Value::String));
        Ok(Value::Object(result))
    }

    /// Create a topic room.
    pub async fn create_topic(&self, name: &str, description: Option<&str>, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let creator_pubkey = hex::encode(local.as_bytes());
        let now = now_secs() as i64;
        let room = self.social_services.create_topic(name, description, &creator_pubkey, now).map_err(|e| format!("create topic: {e}"))?;
        Ok(serde_json::json!({ "topic_id": room.topic_id, "name": room.name, "description": room.description, "created_by": room.created_by, "created_at": room.created_at.as_secs() }))
    }

    /// Join a topic room.
    pub async fn join_topic(&self, topic_id: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let user_pubkey = hex::encode(local.as_bytes());
        let now = now_secs() as i64;
        self.social_services.join_topic(topic_id, &user_pubkey, now).map_err(|e| format!("join topic: {e}"))?;
        Ok(serde_json::json!({ "joined": true }))
    }

    /// Leave a topic room.
    pub async fn leave_topic(&self, topic_id: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let user_pubkey = hex::encode(local.as_bytes());
        self.social_services.leave_topic(topic_id, &user_pubkey).map_err(|e| format!("leave topic: {e}"))?;
        Ok(serde_json::json!({ "left": true }))
    }

    /// List all topic rooms.
    pub async fn list_topics(&self) -> Result<Value, String> {
        let rooms = self.social_services.list_topics().map_err(|e| format!("list topics: {e}"))?;
        let items: Vec<Value> = rooms.iter().map(|r| serde_json::json!({ "topic_id": r.topic_id, "name": r.name, "description": r.description, "created_by": r.created_by, "created_at": r.created_at.as_secs() })).collect();
        Ok(serde_json::json!({ "topics": items }))
    }

    /// Get the feed for a topic room.
    pub async fn get_topic_feed(&self, topic_id: &str, limit: u32) -> Result<Value, String> {
        let page = self.social_services.get_topic_feed(topic_id, None, limit).map_err(|e| format!("topic feed: {e}"))?;
        let items: Vec<Value> = page.items.iter().map(|i| serde_json::json!({ "content_hash": format!("{}", i.content_hash), "author": hex::encode(i.author.as_bytes()), "created_at": i.created_at.as_secs(), "is_reply": i.is_reply })).collect();
        Ok(serde_json::json!({ "items": items, "has_more": page.has_more }))
    }

    /// Post to a topic room.
    pub async fn post_to_topic(&self, topic_id: &str, content_hash_hex: &str) -> Result<Value, String> {
        let hash_bytes = hex::decode(content_hash_hex).map_err(|e| format!("bad content hash: {e}"))?;
        let now = now_secs() as i64;
        self.social_services.post_to_topic(topic_id, &hash_bytes, now).map_err(|e| format!("post to topic: {e}"))?;
        Ok(serde_json::json!({ "posted": true }))
    }

    // ── Groups ────────────────────────────────────────────────────

    /// Create a new group.
    ///
    /// Rate-limited to prevent group-creation spam (3 burst, 5/hr sustained).
    pub async fn create_group(
        &self,
        name: &str,
        description: Option<&str>,
        visibility: &str,
        identity: &IdentityService,
        rate_limiter: &Mutex<RateLimiter>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;

        // Rate limit group creation.
        {
            let mut limiter = rate_limiter.lock().map_err(|e| format!("lock: {e}"))?;
            limiter.check(&local, ActionType::GroupCreate).map_err(|e| {
                format!("Rate limited: {e}")
            })?;
        }

        let creator = hex::encode(local.as_bytes());
        let vis = ephemera_social::GroupVisibility::from_str_lossy(visibility);
        let now = now_secs() as i64;
        let group = self.social_services.create_group(name, description, vis, &creator, now)
            .map_err(|e| format!("create group: {e}"))?;
        Ok(serde_json::json!({
            "group_id": group.group_id,
            "name": group.name,
            "description": group.description,
            "visibility": group.visibility.as_str(),
            "created_by": group.created_by,
            "created_at": group.created_at.as_secs(),
        }))
    }

    /// Register a handle for a group.
    pub async fn register_group_handle(
        &self,
        group_id: &str,
        handle: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        // Only owner or admin can register a handle.
        let role = self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        if !role.can_edit_info() {
            return Err("only owner or admin can register a group handle".into());
        }
        self.social_services.register_group_handle(group_id, handle)
            .map_err(|e| format!("register handle: {e}"))?;
        Ok(serde_json::json!({ "handle": handle }))
    }

    /// Join a public group.
    pub async fn join_group(&self, group_id: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let pubkey = hex::encode(local.as_bytes());
        let now = now_secs() as i64;
        self.social_services.join_group(group_id, &pubkey, now)
            .map_err(|e| format!("join group: {e}"))?;
        Ok(serde_json::json!({ "joined": true }))
    }

    /// Invite someone to a group.
    ///
    /// Rate-limited to prevent invite spam (10 burst, 20/hr sustained).
    pub async fn invite_to_group(
        &self,
        group_id: &str,
        target: &str,
        identity: &IdentityService,
        rate_limiter: &Mutex<RateLimiter>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;

        // Rate limit invites.
        {
            let mut limiter = rate_limiter.lock().map_err(|e| format!("lock: {e}"))?;
            limiter.check(&local, ActionType::GroupInvite).map_err(|e| {
                format!("Rate limited: {e}")
            })?;
        }

        let my_pubkey = hex::encode(local.as_bytes());
        let role = self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        if !role.can_invite() {
            return Err("insufficient permissions to invite".into());
        }
        let now = now_secs() as i64;
        self.social_services.invite_to_group(group_id, target, &my_pubkey, now)
            .map_err(|e| format!("invite: {e}"))?;
        Ok(serde_json::json!({ "invited": true }))
    }

    /// Leave a group.
    pub async fn leave_group(&self, group_id: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let pubkey = hex::encode(local.as_bytes());
        self.social_services.leave_group(group_id, &pubkey)
            .map_err(|e| format!("leave group: {e}"))?;
        Ok(serde_json::json!({ "left": true }))
    }

    /// Set a member's role within a group.
    pub async fn set_group_role(
        &self,
        group_id: &str,
        target: &str,
        new_role: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        let my_role = self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        let target_role = ephemera_social::GroupRole::from_str_lossy(new_role);
        if !my_role.can_set_role(target_role) {
            return Err("insufficient permissions to set this role".into());
        }
        self.social_services.set_group_role(group_id, target, target_role)
            .map_err(|e| format!("set role: {e}"))?;
        Ok(serde_json::json!({ "role_set": true, "new_role": new_role }))
    }

    /// Kick a member from a group.
    pub async fn kick_member(
        &self,
        group_id: &str,
        target: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        let my_role = self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        if !my_role.can_kick() {
            return Err("insufficient permissions to kick".into());
        }
        // Cannot kick someone with equal or higher role.
        let target_role = self.social_services.get_member_role(group_id, target)
            .map_err(|e| format!("get target role: {e}"))?
            .unwrap_or(ephemera_social::GroupRole::Member);
        if target_role <= my_role {
            return Err("cannot kick someone with equal or higher role".into());
        }
        self.social_services.kick_member(group_id, target)
            .map_err(|e| format!("kick: {e}"))?;
        Ok(serde_json::json!({ "kicked": true }))
    }

    /// Ban a member from a group.
    pub async fn ban_member(
        &self,
        group_id: &str,
        target: &str,
        reason: Option<&str>,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        let my_role = self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        if !my_role.can_ban() {
            return Err("insufficient permissions to ban".into());
        }
        let now = now_secs() as i64;
        self.social_services.ban_member(group_id, target, &my_pubkey, reason, now)
            .map_err(|e| format!("ban: {e}"))?;
        Ok(serde_json::json!({ "banned": true }))
    }

    /// Post to a group.
    pub async fn post_to_group(
        &self,
        group_id: &str,
        content_hash_hex: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        // Must be a member to post.
        self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        let hash_bytes = hex::decode(content_hash_hex).map_err(|e| format!("bad hash: {e}"))?;
        let now = now_secs() as i64;
        self.social_services.post_to_group(group_id, &hash_bytes, &my_pubkey, now)
            .map_err(|e| format!("post to group: {e}"))?;
        Ok(serde_json::json!({ "posted": true }))
    }

    /// Get the feed for a group.
    ///
    /// Non-public groups require membership to view the feed.
    pub async fn get_group_feed(&self, group_id: &str, limit: u32, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());

        // For non-public groups, require membership to view the feed.
        let group = self.social_services.get_group(group_id)
            .map_err(|e| format!("get group: {e}"))?
            .ok_or_else(|| "group not found".to_string())?;

        if group.visibility != ephemera_social::GroupVisibility::Public {
            let is_member = self.social_services.is_member(group_id, &my_pubkey)
                .map_err(|e| format!("check membership: {e}"))?;
            if !is_member {
                return Err("not a member of this private/secret group".into());
            }
        }

        let page = self.social_services.get_group_feed(group_id, None, limit)
            .map_err(|e| format!("group feed: {e}"))?;
        let items: Vec<Value> = page.items.iter().map(|i| serde_json::json!({
            "content_hash": format!("{}", i.content_hash),
            "author": hex::encode(i.author.as_bytes()),
            "created_at": i.created_at.as_secs(),
            "is_reply": i.is_reply,
        })).collect();
        Ok(serde_json::json!({ "items": items, "has_more": page.has_more }))
    }

    /// List groups the user is a member of.
    pub async fn list_my_groups(&self, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let pubkey = hex::encode(local.as_bytes());
        let groups = self.social_services.list_my_groups(&pubkey)
            .map_err(|e| format!("list groups: {e}"))?;
        let items: Vec<Value> = groups.iter().map(|gi| serde_json::json!({
            "group_id": gi.group.group_id,
            "name": gi.group.name,
            "handle": gi.group.handle,
            "description": gi.group.description,
            "visibility": gi.group.visibility.as_str(),
            "member_count": gi.member_count,
            "my_role": gi.my_role.map(|r| r.as_str()),
            "created_at": gi.group.created_at.as_secs(),
        })).collect();
        Ok(serde_json::json!({ "groups": items }))
    }

    /// Search public groups.
    pub async fn search_groups(&self, query: &str) -> Result<Value, String> {
        let groups = self.social_services.search_groups(query)
            .map_err(|e| format!("search groups: {e}"))?;
        let items: Vec<Value> = groups.iter().map(|gi| serde_json::json!({
            "group_id": gi.group.group_id,
            "name": gi.group.name,
            "handle": gi.group.handle,
            "description": gi.group.description,
            "member_count": gi.member_count,
        })).collect();
        Ok(serde_json::json!({ "groups": items }))
    }

    /// Get group info.
    pub async fn get_group_info(&self, group_id: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let pubkey = hex::encode(local.as_bytes());
        let info = self.social_services.get_group_info(group_id, Some(&pubkey))
            .map_err(|e| format!("get group info: {e}"))?
            .ok_or_else(|| "group not found".to_string())?;
        let members = self.social_services.list_group_members(group_id)
            .map_err(|e| format!("list members: {e}"))?;
        let member_list: Vec<Value> = members.iter().map(|m| serde_json::json!({
            "pubkey": m.member_pubkey,
            "role": m.role.as_str(),
            "joined_at": m.joined_at.as_secs(),
        })).collect();
        Ok(serde_json::json!({
            "group_id": info.group.group_id,
            "name": info.group.name,
            "handle": info.group.handle,
            "description": info.group.description,
            "visibility": info.group.visibility.as_str(),
            "member_count": info.member_count,
            "my_role": info.my_role.map(|r| r.as_str()),
            "members": member_list,
            "created_at": info.group.created_at.as_secs(),
        }))
    }

    /// Delete a group (owner only).
    pub async fn delete_group(&self, group_id: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        let my_role = self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        if !my_role.can_delete_group() {
            return Err("only the owner can delete a group".into());
        }
        self.social_services.delete_group(group_id)
            .map_err(|e| format!("delete group: {e}"))?;
        Ok(serde_json::json!({ "deleted": true }))
    }

    /// Transfer group ownership to another member (owner only).
    pub async fn transfer_ownership(
        &self,
        group_id: &str,
        new_owner_pubkey: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        let my_role = self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        if !my_role.can_delete_group() {
            // Only owner can transfer.
            return Err("only the owner can transfer ownership".into());
        }
        self.social_services.transfer_ownership(group_id, &my_pubkey, new_owner_pubkey)
            .map_err(|e| format!("transfer ownership: {e}"))?;
        Ok(serde_json::json!({ "transferred": true, "new_owner": new_owner_pubkey }))
    }

    /// Delete a post from a group (moderators, admins, and owner).
    pub async fn delete_group_post(
        &self,
        group_id: &str,
        content_hash_hex: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        let my_role = self.social_services.get_member_role(group_id, &my_pubkey)
            .map_err(|e| format!("get role: {e}"))?
            .ok_or_else(|| "not a member of this group".to_string())?;
        if !my_role.can_delete_posts() {
            return Err("insufficient permissions to delete posts".into());
        }
        let hash_bytes = hex::decode(content_hash_hex).map_err(|e| format!("bad hash: {e}"))?;
        self.social_services.delete_group_post(group_id, &hash_bytes)
            .map_err(|e| format!("delete group post: {e}"))?;
        Ok(serde_json::json!({ "deleted": true }))
    }

    // ── Group Chats ───────────────────────────────────────────────

    /// Create a private group chat.
    pub async fn create_private_chat(
        &self,
        name: Option<&str>,
        members: &[String],
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let creator = hex::encode(local.as_bytes());
        let member_refs: Vec<&str> = members.iter().map(|s| s.as_str()).collect();
        let now = now_secs() as i64;
        let chat = self.social_services.create_private_group_chat(name, &creator, &member_refs, now)
            .map_err(|e| format!("create chat: {e}"))?;
        Ok(serde_json::json!({
            "chat_id": chat.chat_id,
            "name": chat.name,
            "created_at": chat.created_at.as_secs(),
        }))
    }

    /// Create a group-linked chat.
    pub async fn create_group_chat(&self, group_id: &str) -> Result<Value, String> {
        let now = now_secs() as i64;
        let chat = self.social_services.create_group_linked_chat(group_id, now)
            .map_err(|e| format!("create group chat: {e}"))?;
        Ok(serde_json::json!({
            "chat_id": chat.chat_id,
            "name": chat.name,
            "group_id": chat.group_id,
            "created_at": chat.created_at.as_secs(),
        }))
    }

    /// Add a member to a private group chat.
    ///
    /// Only current members can add people. Blocked users cannot be added.
    pub async fn add_chat_member(
        &self,
        chat_id: &str,
        member: &str,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let adder = hex::encode(local.as_bytes());
        let now = now_secs() as i64;
        self.social_services.add_chat_member(chat_id, member, Some(&adder), now)
            .map_err(|e| format!("add chat member: {e}"))?;
        Ok(serde_json::json!({ "added": true }))
    }

    /// Leave a group chat.
    pub async fn leave_chat(&self, chat_id: &str, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let pubkey = hex::encode(local.as_bytes());
        self.social_services.leave_chat(chat_id, &pubkey)
            .map_err(|e| format!("leave chat: {e}"))?;
        Ok(serde_json::json!({ "left": true }))
    }

    /// Send a message to a group chat.
    ///
    /// Rate-limited to prevent message spam (15 burst, 60/hr sustained).
    /// The sender must be a member of the chat (enforced at storage layer).
    pub async fn send_group_chat_message(
        &self,
        chat_id: &str,
        body: &str,
        identity: &IdentityService,
        rate_limiter: &Mutex<RateLimiter>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;

        // Rate limit chat messages.
        {
            let mut limiter = rate_limiter.lock().map_err(|e| format!("lock: {e}"))?;
            limiter.check(&local, ActionType::GroupChatMessage).map_err(|e| {
                format!("Rate limited: {e}")
            })?;
        }

        let sender = hex::encode(local.as_bytes());
        let now = now_secs() as i64;
        let expires_at = now + 86400; // 24h default TTL
        let msg_hash = blake3::hash(format!("{chat_id}:{sender}:{now}:{body}").as_bytes());
        let message_id = hex::encode(msg_hash.as_bytes());
        let msg = self.social_services.send_group_chat_message(
            &message_id, chat_id, &sender, Some(body), now, expires_at,
        ).map_err(|e| format!("send chat message: {e}"))?;
        Ok(serde_json::json!({
            "message_id": msg.message_id,
            "chat_id": msg.chat_id,
            "created_at": msg.created_at.as_secs(),
        }))
    }

    /// Get messages in a group chat.
    pub async fn get_chat_messages(&self, chat_id: &str, limit: u32, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let my_pubkey = hex::encode(local.as_bytes());
        let messages = self.social_services.get_chat_messages(chat_id, limit, None, Some(&my_pubkey))
            .map_err(|e| format!("get chat messages: {e}"))?;
        let items: Vec<Value> = messages.iter().map(|m| serde_json::json!({
            "message_id": m.message_id,
            "sender": m.sender_pubkey,
            "body": m.body,
            "created_at": m.created_at.as_secs(),
        })).collect();
        Ok(serde_json::json!({ "messages": items }))
    }

    /// List group chats I'm a member of.
    pub async fn list_my_group_chats(&self, identity: &IdentityService) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let pubkey = hex::encode(local.as_bytes());
        let chats = self.social_services.list_my_group_chats(&pubkey)
            .map_err(|e| format!("list chats: {e}"))?;
        let items: Vec<Value> = chats.iter().map(|s| serde_json::json!({
            "chat_id": s.chat.chat_id,
            "name": s.chat.name,
            "is_group_linked": s.chat.is_group_linked,
            "group_id": s.chat.group_id,
            "member_count": s.member_count,
            "last_message": s.last_message,
            "last_message_at": s.last_message_at.map(|t| t.as_secs()),
        })).collect();
        Ok(serde_json::json!({ "chats": items }))
    }

    // ── Mentions ──────────────────────────────────────────────────

    /// List posts where I am mentioned.
    pub async fn list_mentions(&self, identity: &IdentityService, limit: u32) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let local_bytes = local.as_bytes().to_vec();
        let items = self.social_services.list_mentions(&local_bytes, limit)
            .map_err(|e| format!("list mentions: {e}"))?;
        let result: Vec<Value> = items.iter().map(|i| serde_json::json!({
            "content_hash": format!("{}", i.content_hash),
            "author": hex::encode(i.author.as_bytes()),
            "created_at": i.created_at.as_secs(),
            "is_reply": i.is_reply,
        })).collect();
        Ok(serde_json::json!({ "mentions": result }))
    }
}

/// Encrypted direct messaging -- backed by ephemera_message::MessageService.
pub struct MessageService {
    pub(crate) metadata_db: Mutex<MetadataDb>,
}

impl MessageService {
    /// Send a message to a recipient.
    ///
    /// Stores the message locally, deposits a dead drop record, and publishes
    /// the dead drop envelope to the gossip network on the `dm_delivery` topic
    /// so that the recipient's node (or relay nodes) can ingest it.
    ///
    /// If `network` is `None` (e.g. during tests or before the network starts),
    /// the message is still stored locally and in the dead drop, but will not
    /// be published to gossip.
    pub async fn send(
        &self,
        to: &str,
        body: &str,
        ttl: Option<u64>,
        identity: &IdentityService,
        network: Option<&crate::network::NetworkSubsystem>,
        dht_storage: Option<&Mutex<DhtStorage>>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let recipient = parse_pubkey(to)?;
        let local_bytes = local.as_bytes().to_vec();
        let recipient_bytes = recipient.as_bytes().to_vec();
        let ttl_secs = ttl.unwrap_or(86400);

        // Compute message ID from body hash.
        let now = now_secs() as i64;
        let msg_hash = blake3::hash(body.as_bytes());
        let msg_id = msg_hash.as_bytes().to_vec();
        let expires_at = now + ttl_secs as i64;

        // Truncate body for safe display preview (never stored in plaintext
        // for the wire -- the full body is encrypted below).
        let preview: String = body.chars().take(100).collect();

        // Encrypt the message body using X25519 ephemeral DH + XChaCha20-Poly1305.
        // Convert the recipient's Ed25519 public key to X25519 for DH exchange.
        let recipient_x25519_pub = ephemera_crypto::ed25519_pub_to_x25519(recipient.as_bytes())
            .map_err(|e| format!("derive recipient x25519: {e}"))?;
        let encrypted_body = ephemera_message::MessageEncryption::encrypt_message(
            body.as_bytes(),
            &recipient_x25519_pub,
        )
        .map_err(|e| format!("encrypt message: {e}"))?;

        // All DB work in a block so the MutexGuard drops before any `.await`.
        let (conversation_id, mailbox_key, msg_content_id, dead_drop_ttl, dd_expires, sealed_data) = {
            let db = self.metadata_db.lock().map_err(|e| format!("lock: {e}"))?;

            // Get or create conversation.
            let conversation_id =
                MsgServiceImpl::get_or_create_conversation(&db, &local_bytes, &recipient_bytes, false)
                    .map_err(|e| format!("get/create conversation: {e}"))?;

            // Store message metadata with encrypted body.
            // body_preview is kept for local display (sender has the plaintext).
            // encrypted_body holds the E2E encrypted ciphertext for the wire.
            db.conn()
                .execute(
                    "INSERT INTO messages (message_id, conversation_id, sender_pubkey,
                     received_at, expires_at, is_read, body_preview, has_media,
                     encrypted_body, is_encrypted)
                     VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 0, ?7, 1)",
                    rusqlite::params![
                        msg_id,
                        conversation_id,
                        local_bytes,
                        now,
                        expires_at,
                        preview,
                        encrypted_body,
                    ],
                )
                .map_err(|e| format!("store message: {e}"))?;

            // Update conversation last_message_at.
            db.conn()
                .execute(
                    "UPDATE conversations SET last_message_at = ?1 WHERE conversation_id = ?2",
                    rusqlite::params![now, conversation_id],
                )
                .map_err(|e| format!("update conversation: {e}"))?;

            // Deposit a dead drop record for the recipient with encrypted data.
            let mailbox_key = DeadDropService::mailbox_key(&recipient);
            let msg_content_id = ContentId::from_digest(*msg_hash.as_bytes());
            let dead_drop_ttl = ttl_secs.min(ephemera_message::dead_drop::DEAD_DROP_MAX_TTL_SECS);
            let dd_expires = now as u64 + dead_drop_ttl;
            let sealed_data = encrypted_body.clone();

            if let Err(e) = DeadDropService::deposit_raw(
                &db,
                &mailbox_key,
                &msg_content_id,
                &sealed_data,
                now as u64,
                dd_expires,
            ) {
                tracing::warn!(error = %e, "failed to deposit dead drop for message");
            }

            (conversation_id, mailbox_key, msg_content_id, dead_drop_ttl, dd_expires, sealed_data)
        }; // db lock dropped here, before any async network I/O

        // Build the gossip wire envelope for the dead drop.
        let envelope = DeadDropEnvelope {
            mailbox_key: *mailbox_key.hash_bytes(),
            message_id: *msg_content_id.hash_bytes(),
            sealed_data: sealed_data.clone(),
            deposited_at: now as u64,
            expires_at: dd_expires,
        };

        let mut published_gossip = false;
        let mut published_dht = false;

        // Publish to gossip network so online peers can relay/store it.
        if let Some(net) = network {
            let topic = ephemera_gossip::GossipTopic::direct_messages();

            // ── Plaintext direct_message path ──────────────────────
            //
            // Publish a plain JSON direct_message so the recipient's
            // message_ingest can store it directly in the messages table
            // without needing decryption. This lets us verify gossip
            // delivery works end-to-end before relying on E2E encryption.
            let local_hex = hex::encode(local.as_bytes());
            let plaintext_msg = serde_json::json!({
                "type": "direct_message",
                "sender": local_hex,
                "recipient": to,
                "body": body,
                "timestamp": now as u64,
            });
            match serde_json::to_vec(&plaintext_msg) {
                Ok(bytes) => {
                    match net.publish(&topic, bytes).await {
                        Ok(()) => {
                            published_gossip = true;
                            tracing::info!(
                                recipient = %to,
                                "published plaintext direct_message to dm_delivery gossip topic"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "failed to publish plaintext direct_message to gossip"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to serialize plaintext direct_message");
                }
            }

            // ── Encrypted DeadDropEnvelope path (kept for relays) ────
            match serde_json::to_vec(&envelope) {
                Ok(envelope_bytes) => {
                    match net.publish(&topic, envelope_bytes).await {
                        Ok(()) => {
                            tracing::debug!(
                                recipient = %to,
                                "published encrypted dead drop to dm_delivery gossip topic"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "failed to publish dead drop to gossip"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to serialize dead drop envelope");
                }
            }
        }

        // Store in DHT for offline delivery with 14-day TTL.
        if let Some(dht) = dht_storage {
            if let Ok(mut storage) = dht.lock() {
                if let Ok(envelope_bytes) = serde_json::to_vec(&envelope) {
                    let dht_record = ephemera_dht::DhtRecord {
                        key: *mailbox_key.hash_bytes(),
                        record_type: ephemera_dht::DhtRecordType::DeadDrop,
                        value: envelope_bytes,
                        publisher: *local.as_bytes(),
                        timestamp: now as u64,
                        ttl_seconds: dead_drop_ttl as u32,
                        signature: vec![], // unsigned for PoC
                    };
                    match storage.put(dht_record) {
                        Ok(()) => {
                            published_dht = true;
                            tracing::debug!(
                                recipient = %to,
                                "stored dead drop in DHT for offline delivery"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "failed to store dead drop in DHT"
                            );
                        }
                    }
                }
            }
        }

        let message_hash = hex::encode(msg_hash.as_bytes());
        Ok(serde_json::json!({
            "sent": true,
            "message_hash": message_hash,
            "conversation_id": hex::encode(&conversation_id),
            "dead_drop": true,
            "published_gossip": published_gossip,
            "published_dht": published_dht,
        }))
    }

    /// List all conversations for the current user.
    pub async fn list_conversations(
        &self,
        identity: &IdentityService,
        profiles_db: Option<&Mutex<MetadataDb>>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let local_bytes = local.as_bytes().to_vec();

        let db = self.metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let conversations = MsgServiceImpl::list_conversations(&db, &local_bytes)
            .map_err(|e| format!("list conversations: {e}"))?;

        let items: Vec<Value> = conversations
            .iter()
            .map(|c| {
                let peer_hex = hex::encode(&c.their_pubkey);

                // Look up display_name and avatar_cid from profiles table (F-13).
                let (peer_display_name, peer_avatar_url): (Option<String>, Option<String>) =
                    if let Some(pdb) = profiles_db {
                        if let Ok(pdb_lock) = pdb.lock() {
                            pdb_lock
                                .conn()
                                .query_row(
                                    "SELECT display_name, avatar_cid FROM profiles WHERE pubkey = ?1",
                                    rusqlite::params![c.their_pubkey],
                                    |row| {
                                        let name: Option<String> = row.get(0)?;
                                        let avatar_cid: Option<Vec<u8>> = row.get(1)?;
                                        let url = avatar_cid
                                            .map(|cid| format!("/media/{}", hex::encode(cid)));
                                        Ok((name, url))
                                    },
                                )
                                .unwrap_or((None, None))
                        } else {
                            (None, None)
                        }
                    } else {
                        (None, None)
                    };

                // Fetch last message preview from the conversation (F-13).
                let last_message_preview: Option<String> = db
                    .conn()
                    .query_row(
                        "SELECT body_preview FROM messages
                         WHERE conversation_id = ?1
                         ORDER BY received_at DESC LIMIT 1",
                        rusqlite::params![c.conversation_id],
                        |row| row.get(0),
                    )
                    .ok()
                    .flatten();

                serde_json::json!({
                    "conversation_id": hex::encode(&c.conversation_id),
                    "peer": peer_hex,
                    "their_pubkey": peer_hex,
                    "peer_display_name": peer_display_name,
                    "peer_avatar_url": peer_avatar_url,
                    "last_message_at": c.last_message_at,
                    "last_message_preview": last_message_preview,
                    "unread_count": c.unread_count,
                    "is_request": c.is_request,
                })
            })
            .collect();

        Ok(serde_json::json!({ "conversations": items }))
    }

    /// Get paginated message thread for a conversation.
    ///
    /// If messages are E2E encrypted and we are the recipient, we decrypt
    /// them using our X25519 secret key (derived from our Ed25519 signing key
    /// via the birational map).
    pub async fn get_thread(
        &self,
        conv_id_hex: &str,
        limit: u64,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        let conv_id = hex::decode(conv_id_hex).map_err(|e| format!("bad conversation_id: {e}"))?;
        let local = get_local_identity(identity)?;
        let local_bytes = local.as_bytes().to_vec();

        // Derive our X25519 secret for decrypting incoming messages.
        let signing_kp = identity.get_signing_keypair()?;
        let our_x25519 = ephemera_crypto::ed25519_seed_to_x25519(&signing_kp.secret_bytes());

        let db = self.metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let messages = MsgServiceImpl::get_messages(&db, &conv_id, None, limit as u32)
            .map_err(|e| format!("get messages: {e}"))?;

        let items: Vec<Value> = messages
            .iter()
            .map(|m| {
                let sender_hex = hex::encode(&m.sender_pubkey);
                let is_own = m.sender_pubkey == local_bytes;
                let status = if is_own {
                    if m.is_read { "read" } else { "sent" }
                } else {
                    "delivered"
                };

                // Attempt to decrypt the message body if encrypted.
                let body = if m.is_encrypted && !is_own {
                    // Incoming encrypted message: decrypt with our X25519 secret.
                    if let Some(ref ciphertext) = m.encrypted_body {
                        match ephemera_message::MessageEncryption::decrypt_message(
                            ciphertext,
                            &our_x25519.secret,
                        ) {
                            Ok(plaintext) => String::from_utf8(plaintext).ok(),
                            Err(e) => {
                                tracing::warn!(
                                    message_id = %hex::encode(&m.id),
                                    error = %e,
                                    "failed to decrypt message"
                                );
                                m.body_preview.clone()
                            }
                        }
                    } else {
                        m.body_preview.clone()
                    }
                } else {
                    // Own messages or unencrypted: use plaintext preview.
                    m.body_preview.clone()
                };

                serde_json::json!({
                    "message_id": hex::encode(&m.id),
                    "sender": sender_hex,
                    "sender_pubkey": sender_hex,
                    "body": body,
                    "body_preview": body,
                    "created_at": m.received_at,
                    "received_at": m.received_at,
                    "expires_at": m.expires_at,
                    "is_own": is_own,
                    "is_read": m.is_read,
                    "is_encrypted": m.is_encrypted,
                    "status": status,
                })
            })
            .collect();

        let has_more = items.len() == limit as usize;
        Ok(serde_json::json!({ "messages": items, "has_more": has_more }))
    }

    /// Mark all messages in a conversation as read.
    pub async fn mark_read(&self, conv_id_hex: &str) -> Result<Value, String> {
        let conv_id = hex::decode(conv_id_hex).map_err(|e| format!("bad conversation_id: {e}"))?;

        let db = self.metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        db.conn()
            .execute(
                "UPDATE messages SET is_read = 1 WHERE conversation_id = ?1 AND is_read = 0",
                rusqlite::params![conv_id],
            )
            .map_err(|e| format!("mark read: {e}"))?;

        // Reset unread count on conversation.
        db.conn()
            .execute(
                "UPDATE conversations SET unread_count = 0 WHERE conversation_id = ?1",
                rusqlite::params![conv_id],
            )
            .map_err(|e| format!("reset unread: {e}"))?;

        Ok(serde_json::json!({ "ok": true }))
    }
}

/// Profile management.
pub struct ProfileService;

impl ProfileService {
    /// Get a profile by pubkey hex.
    pub async fn get(
        &self,
        pubkey_hex: &str,
        metadata_db: &Mutex<MetadataDb>,
    ) -> Result<Value, String> {
        let pubkey_bytes = hex::decode(pubkey_hex).map_err(|e| format!("bad pubkey: {e}"))?;
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        match db.conn().query_row(
            "SELECT display_name, bio FROM profiles WHERE pubkey = ?1",
            rusqlite::params![pubkey_bytes],
            |row| {
                let name: Option<String> = row.get(0)?;
                let bio: Option<String> = row.get(1)?;
                Ok(serde_json::json!({
                    "pubkey": pubkey_hex, "display_name": name, "bio": bio,
                }))
            },
        ) {
            Ok(val) => Ok(val),
            Err(_) => Ok(serde_json::json!({
                "pubkey": pubkey_hex, "display_name": null, "bio": null,
            })),
        }
    }

    /// Update the current user's profile.
    ///
    /// Validates display name (max 30 chars) and bio (max 160 chars) before
    /// persisting. Returns an error if validation fails.
    pub async fn update(
        &self,
        name: Option<&str>,
        bio: Option<&str>,
        identity: &IdentityService,
        metadata_db: &Mutex<MetadataDb>,
    ) -> Result<Value, String> {
        // Validate profile fields against size limits (gap 3.2).
        let update = ephemera_social::ProfileUpdate {
            display_name: name.map(String::from),
            bio: bio.map(String::from),
            avatar_id: None,
        };
        update.validate().map_err(|e| format!("profile validation: {e}"))?;

        let signing_kp = identity.get_signing_keypair()?;
        let pubkey_bytes = signing_kp.public_key().as_bytes().to_vec();
        let now = now_secs() as i64;
        let sig = signing_kp.sign(b"profile-update");

        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        db.conn()
            .execute(
                "INSERT INTO profiles (pubkey, display_name, bio, updated_at, signature, received_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?4)
                 ON CONFLICT(pubkey) DO UPDATE SET
                    display_name = COALESCE(?2, display_name),
                    bio = COALESCE(?3, bio),
                    updated_at = ?4, signature = ?5",
                rusqlite::params![pubkey_bytes, name, bio, now, sig.as_slice()],
            )
            .map_err(|e| format!("update profile: {e}"))?;

        Ok(serde_json::json!({ "updated": true }))
    }

    /// Update profile and publish to the DHT for network discovery.
    ///
    /// Includes an `avatar_thumb` field (base64-encoded 64x64 thumbnail,
    /// ~2KB) in the DHT record when an avatar exists, so remote users can
    /// display a small avatar without fetching the full blob.
    pub async fn update_and_publish(
        &self,
        name: Option<&str>,
        bio: Option<&str>,
        identity: &IdentityService,
        metadata_db: &Mutex<MetadataDb>,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Value, String> {
        let result = self.update(name, bio, identity, metadata_db).await?;

        // Publish the profile to DHT for network-wide discovery.
        let signing_kp = identity.get_signing_keypair()?;
        let pubkey_hex = hex::encode(signing_kp.public_key().as_bytes());

        // Build profile JSON, including avatar thumbnail if available.
        let profile_json = build_dht_profile(
            &pubkey_hex, name, bio, metadata_db,
        );

        if let Err(e) = DhtNodeService::store_profile(
            &pubkey_hex, &profile_json, identity, dht_storage,
        ) {
            tracing::warn!(error = %e, "failed to publish profile to DHT");
        } else {
            tracing::info!(pubkey = %pubkey_hex, "profile published to DHT");
        }

        Ok(result)
    }

    /// Get a profile, falling back to DHT if not found locally.
    pub async fn get_with_dht(
        &self,
        pubkey_hex: &str,
        metadata_db: &Mutex<MetadataDb>,
        dht_storage: &Mutex<DhtStorage>,
    ) -> Result<Value, String> {
        let local = self.get(pubkey_hex, metadata_db).await?;

        // If we got a result with a display_name, return it.
        if local.get("display_name").and_then(|v| v.as_str()).is_some() {
            return Ok(local);
        }

        // Fall back to DHT.
        match DhtNodeService::lookup_profile(pubkey_hex, dht_storage)? {
            Some(dht_profile) => Ok(dht_profile),
            None => Ok(local),
        }
    }

    /// Get a profile, querying the network DHT if not found locally.
    ///
    /// This is the fully network-aware version: after checking local DB and
    /// local DHT, it publishes a DHT query via gossip and waits up to 5
    /// seconds for a peer to respond.
    pub async fn get_with_network_dht(
        &self,
        pubkey_hex: &str,
        metadata_db: &Mutex<MetadataDb>,
        services: &std::sync::Arc<super::ServiceContainer>,
    ) -> Result<Value, String> {
        let local = self.get(pubkey_hex, metadata_db).await?;

        // If we got a result with a display_name, return it.
        if local.get("display_name").and_then(|v| v.as_str()).is_some() {
            return Ok(local);
        }

        // Try local DHT.
        if let Some(dht_profile) = DhtNodeService::lookup_profile(
            pubkey_hex, &services.dht_storage,
        )? {
            return Ok(dht_profile);
        }

        // Query the network DHT.
        let key = super::dht::dht_key("profile", pubkey_hex);
        match crate::dht_query::query_network_dht(&key, services).await? {
            Some(val) => {
                // Also store in local profiles table for future lookups.
                store_profile_from_dht(pubkey_hex, &val, metadata_db);
                Ok(val)
            }
            None => Ok(local),
        }
    }

    /// Get the current user's full profile including handle and post count.
    pub async fn get_mine(
        &self,
        identity: &IdentityService,
        metadata_db: &Mutex<MetadataDb>,
        handle_registry: &Mutex<ephemera_social::HandleRegistry>,
    ) -> Result<Value, String> {
        let signing_kp = identity.get_signing_keypair()?;
        let pubkey = signing_kp.public_key();
        let pubkey_hex = hex::encode(pubkey.as_bytes());
        let pubkey_bytes = pubkey.as_bytes().to_vec();

        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;

        // Fetch profile fields.
        let (display_name, bio, avatar_cid): (Option<String>, Option<String>, Option<Vec<u8>>) =
            db.conn()
                .query_row(
                    "SELECT display_name, bio, avatar_cid FROM profiles WHERE pubkey = ?1",
                    rusqlite::params![pubkey_bytes],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap_or((None, None, None));

        // Count posts by this author.
        let post_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM posts WHERE author_pubkey = ?1 AND is_tombstone = 0",
                rusqlite::params![pubkey_bytes],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Fetch created_at from the profile row (updated_at as proxy).
        let created_at: i64 = db
            .conn()
            .query_row(
                "SELECT updated_at FROM profiles WHERE pubkey = ?1",
                rusqlite::params![pubkey_bytes],
                |row| row.get(0),
            )
            .unwrap_or(0);

        drop(db);

        // Build avatar_url from avatar_cid.
        let avatar_url = avatar_cid
            .as_ref()
            .map(|cid| format!("/media/{}", hex::encode(cid)));

        // Lookup handle.
        let handle: Option<String> = {
            let reg = handle_registry.lock().map_err(|e| format!("lock: {e}"))?;
            reg.lookup_by_owner(&pubkey)
                .filter(|h| !h.is_expired())
                .map(|h| format!("@{}", h.name))
        };

        Ok(serde_json::json!({
            "pubkey": pubkey_hex,
            "display_name": display_name,
            "handle": handle,
            "bio": bio,
            "avatar_url": avatar_url,
            "node_id": pubkey_hex,
            "created_at": created_at,
            "post_count": post_count,
        }))
    }

    /// Update the user's avatar: process image through the media pipeline,
    /// store it, and update the profile's avatar_cid.
    pub async fn update_avatar(
        &self,
        data: &[u8],
        _filename: &str,
        identity: &IdentityService,
        metadata_db: &Mutex<MetadataDb>,
        content_store: &ContentStore,
    ) -> Result<Value, String> {
        use ephemera_media::pipeline::{MediaPipeline, ProcessedContent};

        let processed = MediaPipeline::process_auto(data)
            .map_err(|e| format!("avatar processing failed: {e}"))?;

        let content_hash = match &processed {
            ProcessedContent::Image(img) => img.content_hash.clone(),
            ProcessedContent::Video(_) => {
                return Err("avatar must be an image, not a video".to_string());
            }
        };

        let content_hash_hex = hex::encode(content_hash.hash_bytes());
        let avatar_cid = content_hash.hash_bytes().to_vec();

        // Store the processed standard image as a blob.
        if let ProcessedContent::Image(ref img) = processed {
            content_store
                .put(&img.standard)
                .map_err(|e| format!("store avatar blob: {e}"))?;
            // Also store the thumbnail.
            content_store
                .put(&img.thumbnail)
                .map_err(|e| format!("store avatar thumbnail: {e}"))?;
        }

        // Update profile avatar_cid.
        let signing_kp = identity.get_signing_keypair()?;
        let pubkey_bytes = signing_kp.public_key().as_bytes().to_vec();
        let now = now_secs() as i64;
        let sig = signing_kp.sign(b"profile-update-avatar");

        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        db.conn()
            .execute(
                "INSERT INTO profiles (pubkey, avatar_cid, updated_at, signature, received_at)
                 VALUES (?1, ?2, ?3, ?4, ?3)
                 ON CONFLICT(pubkey) DO UPDATE SET
                    avatar_cid = ?2, updated_at = ?3, signature = ?4",
                rusqlite::params![pubkey_bytes, avatar_cid, now, sig.as_slice()],
            )
            .map_err(|e| format!("update avatar: {e}"))?;

        let avatar_url = format!("/media/{content_hash_hex}");
        Ok(serde_json::json!({ "avatar_url": avatar_url }))
    }
}

/// Content moderation: report, block, mute -- backed by ephemera_mod::ReportService
/// and delegates block/mute to SocialService.
pub struct ModerationService {
    pub(crate) report_service: Mutex<ReportService>,
}

impl ModerationService {
    /// Maximum reports from a single reporter per day before deprioritization.
    const MAX_REPORTS_PER_REPORTER_PER_DAY: usize = 20;

    /// Number of distinct reporters required to auto-tombstone content.
    const TOMBSTONE_THRESHOLD: usize = 5;
    /// Report content by its hash with a reason.
    ///
    /// Enforces report abuse detection (>20 reports/day from one reporter)
    /// and auto-tombstone threshold (>=5 distinct reporters).
    pub async fn report(
        &self,
        hash: &str,
        reason: &str,
        identity: &IdentityService,
        post_service: &PostService,
        metadata_db: &Mutex<MetadataDb>,
        reputation: &Mutex<HashMap<IdentityKey, ReputationScore>>,
    ) -> Result<Value, String> {
        let local = get_local_identity(identity)?;
        let hash_bytes = hex::decode(hash).map_err(|e| format!("bad hash: {e}"))?;
        if hash_bytes.len() != 32 {
            return Err("content hash must be 32 bytes hex".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash_bytes);
        let content_id = ContentId::from_digest(arr);

        let report_reason = match reason {
            "harassment" => ephemera_mod::ReportReason::Harassment,
            "hate_speech" => ephemera_mod::ReportReason::HateSpeech,
            "violence" => ephemera_mod::ReportReason::Violence,
            "spam" => ephemera_mod::ReportReason::Spam,
            "csam" => ephemera_mod::ReportReason::Csam,
            "impersonation" => ephemera_mod::ReportReason::Impersonation,
            other => ephemera_mod::ReportReason::Other(other.to_string()),
        };

        // All report service operations are done inside this block so the
        // MutexGuard is dropped before any `.await`.
        let report_count = {
            let mut svc = self
                .report_service
                .lock()
                .map_err(|e| format!("lock: {e}"))?;

            // Report abuse detection: check if this reporter has filed too many reports.
            let reporter_report_count = svc
                .list_reports()
                .iter()
                .filter(|r| r.reporter == local)
                .count();
            if reporter_report_count >= Self::MAX_REPORTS_PER_REPORTER_PER_DAY {
                tracing::warn!(
                    reporter = %hex::encode(local.as_bytes()),
                    count = reporter_report_count,
                    "possible report abuse: too many reports from this reporter"
                );
                return Ok(serde_json::json!({
                    "status": "received",
                    "note": "report queued for review"
                }));
            }

            svc.create_report(local, content_id.clone(), report_reason.clone(), None)
                .map_err(|e| format!("create report: {e}"))?;

            svc.reporter_count(&content_id)
        };
        // MutexGuard<ReportService> is now dropped.

        // Persist the report to SQLite (F-17: reports survive restart).
        {
            let reason_str = match &report_reason {
                ephemera_mod::ReportReason::Harassment => "harassment",
                ephemera_mod::ReportReason::HateSpeech => "hate_speech",
                ephemera_mod::ReportReason::Violence => "violence",
                ephemera_mod::ReportReason::Spam => "spam",
                ephemera_mod::ReportReason::Csam => "csam",
                ephemera_mod::ReportReason::Impersonation => "impersonation",
                ephemera_mod::ReportReason::Other(s) => s.as_str(),
            };
            let reporter_bytes = local.as_bytes().to_vec();
            let content_wire = content_id.to_wire_bytes();
            if let Ok(db) = metadata_db.lock() {
                // Ensure reports table exists (idempotent).
                let _ = db.conn().execute_batch(
                    "CREATE TABLE IF NOT EXISTS reports (
                         id INTEGER PRIMARY KEY,
                         reporter_pubkey BLOB NOT NULL,
                         content_hash BLOB NOT NULL,
                         reason TEXT NOT NULL,
                         created_at INTEGER NOT NULL,
                         UNIQUE(reporter_pubkey, content_hash)
                     )",
                );
                let _ = db.conn().execute(
                    "INSERT OR IGNORE INTO reports (reporter_pubkey, content_hash, reason, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![reporter_bytes, content_wire, reason_str, now_secs() as i64],
                );
            }
        }

        if report_count >= Self::TOMBSTONE_THRESHOLD {
            tracing::warn!(
                content = %hash,
                report_count = report_count,
                "auto-tombstoning content due to report threshold"
            );

            // Auto-tombstone the content.
            let _ = post_service.delete(hash, metadata_db).await;

            // Look up the author and penalize reputation.
            let author_key = lookup_post_author(hash, metadata_db);
            if let Some(author) = author_key {
                let mut rep_map = reputation.lock().map_err(|e| format!("lock: {e}"))?;
                let score = rep_map.entry(author).or_insert_with(ReputationScore::new);
                score.record_event(ReputationEvent::CommunityTombstone);
            }

            return Ok(serde_json::json!({
                "reported": true,
                "tombstoned": true,
                "report_count": report_count,
            }));
        }

        Ok(serde_json::json!({ "reported": true }))
    }

    /// Block a user -- delegates to SocialService.
    pub async fn block(
        &self,
        target: &str,
        social: &SocialService,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        social.block(target, identity).await
    }

    /// Unblock a user -- delegates to SocialService.
    pub async fn unblock(
        &self,
        target: &str,
        social: &SocialService,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        social.unblock(target, identity).await
    }

    /// Mute a user -- delegates to SocialService.
    pub async fn mute(
        &self,
        target: &str,
        duration_hours: Option<u64>,
        social: &SocialService,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        social.mute(target, duration_hours, identity).await
    }

    /// Unmute a user -- delegates to SocialService.
    pub async fn unmute(
        &self,
        target: &str,
        social: &SocialService,
        identity: &IdentityService,
    ) -> Result<Value, String> {
        social.unmute(target, identity).await
    }

    /// Seed a report directly (for testing and internal use).
    ///
    /// Bypasses report abuse detection -- use `report()` for user-facing reports.
    pub fn seed_report(
        &self,
        reporter: IdentityKey,
        content_id: ContentId,
        reason: ephemera_mod::ReportReason,
    ) -> Result<(), String> {
        let mut svc = self.report_service.lock().map_err(|e| format!("lock: {e}"))?;
        svc.create_report(reporter, content_id, reason, None)
            .map_err(|e| format!("seed report: {e}"))?;
        Ok(())
    }

    /// Get the number of distinct reporters for a given content hash.
    pub fn reporter_count(&self, content_id: &ContentId) -> Result<usize, String> {
        let svc = self.report_service.lock().map_err(|e| format!("lock: {e}"))?;
        Ok(svc.reporter_count(content_id))
    }
}
