//! Session manager for Double Ratchet sessions.
//!
//! Manages per-conversation ratchet state with SQLite persistence.
//! Each conversation has exactly one `RatchetState` that is loaded
//! from disk on access and saved after each encrypt/decrypt operation.
//!
//! On first contact with a peer, X3DH key exchange is performed to
//! establish the shared secret that seeds the ratchet.

use ephemera_crypto::{X25519KeyPair, X25519PublicKey};
use ephemera_store::MetadataDb;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::prekey::PrekeyBundle;
use crate::ratchet::{MessageHeader, RatchetState};
use crate::x3dh::{x3dh_initiate, X3dhInitialMessage};
use crate::MessageError;

/// Result of encrypting a message through the session manager.
pub struct EncryptedMessage {
    /// The ratchet message header (ratchet pubkey + counters).
    pub header: MessageHeader,
    /// The encrypted ciphertext.
    pub ciphertext: Vec<u8>,
    /// If this is the first message, contains the X3DH initial header.
    pub x3dh_header: Option<X3dhInitialMessage>,
}

/// Manages ratchet sessions per conversation, backed by SQLite.
pub struct SessionManager;

impl SessionManager {
    /// Load an existing ratchet session from SQLite.
    ///
    /// Returns `None` if no session exists for this conversation.
    pub fn load_session(
        db: &MetadataDb,
        conversation_id: &str,
    ) -> Result<Option<RatchetState>, MessageError> {
        let mut stmt = db.conn().prepare(
            "SELECT root_key, send_chain_key, recv_chain_key,
                    send_ratchet_secret, recv_ratchet_pubkey,
                    send_count, recv_count, prev_send_count
             FROM ratchet_sessions WHERE conversation_id = ?1",
        )?;

        let result = stmt.query_row(rusqlite::params![conversation_id], |row| {
            let root_key: Vec<u8> = row.get(0)?;
            let send_ck: Vec<u8> = row.get(1)?;
            let recv_ck: Vec<u8> = row.get(2)?;
            let send_secret: Vec<u8> = row.get(3)?;
            let recv_pub: Option<Vec<u8>> = row.get(4)?;
            let send_count: u32 = row.get(5)?;
            let recv_count: u32 = row.get(6)?;
            let prev_send: u32 = row.get(7)?;
            Ok((
                root_key, send_ck, recv_ck, send_secret, recv_pub, send_count,
                recv_count, prev_send,
            ))
        });

        match result {
            Ok((root_key, send_ck, recv_ck, send_secret, recv_pub, sc, rc, ps)) => {
                let rk = to_array32(&root_key)?;
                let sck = to_array32(&send_ck)?;
                let rck = to_array32(&recv_ck)?;
                let ss = to_array32(&send_secret)?;
                let rpk = match recv_pub {
                    Some(bytes) => Some(X25519PublicKey::from_bytes(to_array32(&bytes)?)),
                    None => None,
                };
                Ok(Some(RatchetState::from_persisted(
                    rk, sck, rck, &ss, rpk, sc, rc, ps,
                )))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(MessageError::Database(e)),
        }
    }

    /// Save (upsert) a ratchet session to SQLite.
    pub fn save_session(
        db: &MetadataDb,
        conversation_id: &str,
        peer_pubkey: &[u8],
        state: &RatchetState,
    ) -> Result<(), MessageError> {
        let now = now_secs();
        let recv_pub = state
            .get_recv_ratchet_pubkey()
            .map(|k| k.as_bytes().to_vec());

        db.conn().execute(
            "INSERT INTO ratchet_sessions
             (conversation_id, peer_pubkey, root_key, send_chain_key,
              recv_chain_key, send_ratchet_secret, send_ratchet_pubkey,
              recv_ratchet_pubkey, send_count, recv_count,
              prev_send_count, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
             ON CONFLICT(conversation_id) DO UPDATE SET
                root_key = ?3, send_chain_key = ?4, recv_chain_key = ?5,
                send_ratchet_secret = ?6, send_ratchet_pubkey = ?7,
                recv_ratchet_pubkey = ?8, send_count = ?9, recv_count = ?10,
                prev_send_count = ?11, updated_at = ?12",
            rusqlite::params![
                conversation_id,
                peer_pubkey,
                state.root_key().as_slice(),
                state.get_send_chain_key().as_slice(),
                state.get_recv_chain_key().as_slice(),
                state.send_ratchet_secret().as_slice(),
                state.public_ratchet_key().as_bytes().as_slice(),
                recv_pub,
                state.get_send_count(),
                state.get_recv_count(),
                state.get_previous_send_count(),
                now,
            ],
        )?;

        Ok(())
    }

    /// Encrypt a message for a peer, creating a new session via X3DH
    /// if one does not already exist.
    ///
    /// Returns the encrypted message with header and optional X3DH data.
    pub fn encrypt_for_peer(
        db: &MetadataDb,
        conversation_id: &str,
        peer_pubkey: &[u8],
        our_x25519: &X25519KeyPair,
        our_identity: &ephemera_types::IdentityKey,
        their_bundle: Option<&PrekeyBundle>,
        plaintext: &[u8],
    ) -> Result<EncryptedMessage, MessageError> {
        let existing = Self::load_session(db, conversation_id)?;

        let (mut state, x3dh_header) = match existing {
            Some(state) => (state, None),
            None => {
                // First message: perform X3DH to establish session.
                let bundle = their_bundle.ok_or_else(|| MessageError::X3dhFailed {
                    reason: "no prekey bundle for new session".into(),
                })?;
                let result = x3dh_initiate(
                    &our_x25519.secret,
                    &our_x25519.public,
                    our_identity,
                    bundle,
                )?;
                let ratchet = RatchetState::init_sender(
                    &result.shared_secret,
                    &bundle.signed_prekey,
                )?;
                (ratchet, Some(result.initial_message.clone()))
            }
        };

        let (header, ciphertext) = state.encrypt_message(plaintext)?;
        Self::save_session(db, conversation_id, peer_pubkey, &state)?;

        Ok(EncryptedMessage {
            header,
            ciphertext,
            x3dh_header,
        })
    }

    /// Decrypt a message from a peer, loading (or creating) the session.
    ///
    /// If no session exists and `shared_secret` is provided (from X3DH
    /// response), a new receiver session is initialized.
    pub fn decrypt_from_peer(
        db: &MetadataDb,
        conversation_id: &str,
        peer_pubkey: &[u8],
        our_ratchet_key: Option<X25519KeyPair>,
        shared_secret: Option<&[u8; 32]>,
        header: &MessageHeader,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, MessageError> {
        let existing = Self::load_session(db, conversation_id)?;

        let mut state = match existing {
            Some(state) => state,
            None => {
                // First received message: need shared_secret from X3DH.
                let secret = shared_secret.ok_or_else(|| MessageError::RatchetError {
                    reason: "no session and no shared secret for init".into(),
                })?;
                let ratchet_key = our_ratchet_key.ok_or_else(|| MessageError::RatchetError {
                    reason: "no ratchet key for receiver init".into(),
                })?;
                RatchetState::init_receiver(secret, ratchet_key)
            }
        };

        let plaintext = state.decrypt_message(header, ciphertext)?;
        Self::save_session(db, conversation_id, peer_pubkey, &state)?;

        Ok(plaintext)
    }

    /// Check whether a session exists for the given conversation.
    pub fn has_session(db: &MetadataDb, conversation_id: &str) -> Result<bool, MessageError> {
        let count: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM ratchet_sessions WHERE conversation_id = ?1",
            rusqlite::params![conversation_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

/// Convert a byte vector to a 32-byte array.
fn to_array32(bytes: &[u8]) -> Result<[u8; 32], MessageError> {
    if bytes.len() != 32 {
        return Err(MessageError::RatchetError {
            reason: format!("expected 32 bytes, got {}", bytes.len()),
        });
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(bytes);
    Ok(arr)
}

/// Return the current wall-clock time as Unix seconds.
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
