//! Invite link generation and QR code payload utilities.
//!
//! Provides helpers for out-of-band connection establishment:
//! - **Invite links**: `ephemera://connect/<hex_pubkey>` URLs that can be
//!   shared via any channel to initiate a connection request.
//! - **QR payloads**: JSON blobs containing a pubkey and session nonce for
//!   in-person pairing.

use ephemera_types::IdentityKey;

/// URI scheme for invite links.
const INVITE_PREFIX: &str = "ephemera://connect/";

/// Error type for invite link and QR operations.
#[derive(Debug, thiserror::Error)]
pub enum InviteError {
    /// The invite link has an invalid format.
    #[error("invalid invite link format: {reason}")]
    InvalidFormat {
        /// Human-readable reason.
        reason: String,
    },

    /// The QR payload could not be parsed.
    #[error("invalid QR payload: {reason}")]
    InvalidQrPayload {
        /// Human-readable reason.
        reason: String,
    },
}

/// Generate an invite link from an identity key.
///
/// The link format is `ephemera://connect/<hex_pubkey>` where the pubkey is
/// the 32-byte Ed25519 public key encoded as lowercase hex (64 chars).
#[must_use]
pub fn generate_invite_link(identity: &IdentityKey) -> String {
    format!("{INVITE_PREFIX}{}", hex::encode(identity.as_bytes()))
}

/// Parse an invite link and extract the identity key.
///
/// # Errors
///
/// Returns [`InviteError::InvalidFormat`] if the link does not match the
/// expected `ephemera://connect/<hex_pubkey>` format.
pub fn parse_invite_link(link: &str) -> Result<IdentityKey, InviteError> {
    let pubkey_hex =
        link.strip_prefix(INVITE_PREFIX)
            .ok_or_else(|| InviteError::InvalidFormat {
                reason: format!("link must start with '{INVITE_PREFIX}'"),
            })?;

    let pubkey_bytes = hex::decode(pubkey_hex).map_err(|e| InviteError::InvalidFormat {
        reason: format!("invalid hex in pubkey: {e}"),
    })?;

    if pubkey_bytes.len() != 32 {
        return Err(InviteError::InvalidFormat {
            reason: format!("pubkey must be 32 bytes, got {}", pubkey_bytes.len()),
        });
    }

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&pubkey_bytes);
    Ok(IdentityKey::from_bytes(arr))
}

/// JSON structure for QR code payloads used in in-person pairing.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct QrPayload {
    /// Hex-encoded Ed25519 public key.
    pub pubkey: String,
    /// Random session nonce (hex-encoded, 16 bytes / 32 hex chars).
    pub nonce: String,
}

/// Generate a QR code payload for in-person pairing.
///
/// Returns a JSON string containing the pubkey and a fresh random session nonce.
pub fn generate_qr_payload(identity: &IdentityKey) -> String {
    let mut nonce_bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce_bytes);

    let payload = QrPayload {
        pubkey: hex::encode(identity.as_bytes()),
        nonce: hex::encode(nonce_bytes),
    };

    // Serialization of a simple struct will not fail.
    serde_json::to_string(&payload).expect("QrPayload serialization cannot fail")
}

/// Parse a QR code payload and extract the identity key.
///
/// # Errors
///
/// Returns [`InviteError::InvalidQrPayload`] if the JSON is malformed or
/// the pubkey is not a valid 32-byte hex string.
pub fn parse_qr_payload(json: &str) -> Result<IdentityKey, InviteError> {
    let payload: QrPayload =
        serde_json::from_str(json).map_err(|e| InviteError::InvalidQrPayload {
            reason: format!("invalid JSON: {e}"),
        })?;

    let pubkey_bytes = hex::decode(&payload.pubkey).map_err(|e| InviteError::InvalidQrPayload {
        reason: format!("invalid hex in pubkey: {e}"),
    })?;

    if pubkey_bytes.len() != 32 {
        return Err(InviteError::InvalidQrPayload {
            reason: format!("pubkey must be 32 bytes, got {}", pubkey_bytes.len()),
        });
    }

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&pubkey_bytes);
    Ok(IdentityKey::from_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_identity() -> IdentityKey {
        IdentityKey::from_bytes([0xAA; 32])
    }

    #[test]
    fn invite_link_roundtrip() {
        let identity = test_identity();
        let link = generate_invite_link(&identity);
        assert!(link.starts_with(INVITE_PREFIX));
        let recovered = parse_invite_link(&link).unwrap();
        assert_eq!(recovered, identity);
    }

    #[test]
    fn invite_link_bad_prefix() {
        let result = parse_invite_link("https://example.com/connect/aabb");
        assert!(result.is_err());
    }

    #[test]
    fn invite_link_bad_hex() {
        let result = parse_invite_link("ephemera://connect/not-hex!");
        assert!(result.is_err());
    }

    #[test]
    fn invite_link_wrong_length() {
        let result = parse_invite_link("ephemera://connect/aabb");
        assert!(result.is_err());
    }

    #[test]
    fn qr_payload_roundtrip() {
        let identity = test_identity();
        let json = generate_qr_payload(&identity);
        let recovered = parse_qr_payload(&json).unwrap();
        assert_eq!(recovered, identity);
    }

    #[test]
    fn qr_payload_bad_json() {
        let result = parse_qr_payload("{invalid json}");
        assert!(result.is_err());
    }

    #[test]
    fn qr_payload_contains_nonce() {
        let identity = test_identity();
        let json = generate_qr_payload(&identity);
        let payload: QrPayload = serde_json::from_str(&json).unwrap();
        // Nonce should be 32 hex chars (16 bytes).
        assert_eq!(payload.nonce.len(), 32);
    }
}
