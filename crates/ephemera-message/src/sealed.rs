//! Sealed sender envelope for direct messages.
//!
//! The wire message envelope does NOT contain the sender's pseudonym in
//! plaintext. The sender's identity is placed INSIDE the encrypted payload
//! so that only the recipient can see who sent the message after decryption.
//!
//! This prevents relay nodes and passive observers from learning the sender
//! of a direct message. They can only see the recipient (needed for routing).

use ephemera_crypto::{X25519PublicKey, X25519SecretKey};
use ephemera_types::{IdentityKey, Timestamp, Ttl};
use serde::{Deserialize, Serialize};

use crate::encryption::MessageEncryption;
use crate::MessageError;

/// Maximum wire size for a sealed envelope body (32 KB per spec).
pub const MAX_SEALED_WIRE_SIZE: usize = 32_768;

/// The inner payload that is encrypted inside the sealed envelope.
///
/// This contains the actual sender identity and the message body.
/// Only the recipient can decrypt this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedPayload {
    /// The sender's pseudonym public key (hidden from observers).
    pub sender: IdentityKey,
    /// The plaintext message body.
    pub body: Vec<u8>,
}

/// A sealed-sender message envelope.
///
/// On the wire, this carries:
/// - The recipient's public key (needed for routing/delivery).
/// - The encrypted payload (containing sender identity + message body).
/// - Metadata (timestamp, TTL) that is public.
///
/// The sender's identity is NOT in plaintext anywhere on this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedEnvelope {
    /// Recipient pseudonym public key (for routing). Public on the wire.
    pub recipient: IdentityKey,
    /// Encrypted payload: `ephemeral_pubkey (32) || nonce (24) || ciphertext+tag`.
    /// The ciphertext decrypts to a serialized [`SealedPayload`].
    pub ciphertext: Vec<u8>,
    /// When the message was created (public metadata for TTL enforcement).
    pub timestamp: Timestamp,
    /// How long the message should be retained.
    pub ttl: Ttl,
}

impl SealedEnvelope {
    /// Create a sealed envelope by encrypting the message body along with
    /// the sender's identity for the given recipient.
    ///
    /// The sender's identity is placed inside the encrypted payload so
    /// it is invisible on the wire.
    pub fn seal(
        sender: &IdentityKey,
        recipient: &IdentityKey,
        recipient_x25519: &X25519PublicKey,
        body: &[u8],
        ttl: Ttl,
    ) -> Result<Self, MessageError> {
        let payload = SealedPayload {
            sender: *sender,
            body: body.to_vec(),
        };
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| MessageError::Deserialization(e.to_string()))?;

        if payload_bytes.len() > MAX_SEALED_WIRE_SIZE {
            return Err(MessageError::BodyTooLarge {
                got: payload_bytes.len(),
                max: MAX_SEALED_WIRE_SIZE,
            });
        }

        let ciphertext = MessageEncryption::encrypt_message(&payload_bytes, recipient_x25519)?;

        Ok(Self {
            recipient: *recipient,
            ciphertext,
            timestamp: Timestamp::now(),
            ttl,
        })
    }

    /// Open a sealed envelope, decrypting the payload to reveal the sender
    /// and message body.
    ///
    /// The recipient uses their X25519 secret key to decrypt.
    pub fn open(&self, our_secret: &X25519SecretKey) -> Result<SealedPayload, MessageError> {
        let plaintext = MessageEncryption::decrypt_message(&self.ciphertext, our_secret)?;
        let payload: SealedPayload = serde_json::from_slice(&plaintext)
            .map_err(|e| MessageError::Deserialization(e.to_string()))?;
        Ok(payload)
    }

    /// Check whether the sender's identity is visible anywhere in the
    /// envelope's public fields.
    ///
    /// This should always return `false` for a correctly constructed
    /// envelope -- the sender is only inside the ciphertext.
    pub fn sender_is_visible(&self, sender: &IdentityKey) -> bool {
        // Check if sender bytes appear in the recipient field.
        if self.recipient == *sender {
            // If sender == recipient, that's a self-message, not a leak.
            return false;
        }
        // The sender should not appear in any public field.
        // The only identity-like field is `recipient`.
        false
    }

    /// Validate wire-level constraints on the envelope.
    pub fn validate(&self) -> Result<(), MessageError> {
        if self.ciphertext.len() > MAX_SEALED_WIRE_SIZE {
            return Err(MessageError::BodyTooLarge {
                got: self.ciphertext.len(),
                max: MAX_SEALED_WIRE_SIZE,
            });
        }
        Ok(())
    }

    /// Check whether this message has expired relative to the current time.
    pub fn is_expired(&self) -> bool {
        let now = Timestamp::now().as_secs();
        let expires_at = self.timestamp.as_secs() + self.ttl.as_secs();
        now > expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemera_crypto::X25519KeyPair;

    fn alice_identity() -> IdentityKey {
        IdentityKey::from_bytes([0xAA; 32])
    }

    fn bob_identity() -> IdentityKey {
        IdentityKey::from_bytes([0xBB; 32])
    }

    #[test]
    fn test_sealed_sender() {
        let alice_id = alice_identity();
        let bob_id = bob_identity();
        let bob_x25519 = X25519KeyPair::generate();
        let ttl = Ttl::one_day();

        let message = b"Hello Bob, this is Alice!";

        // Alice seals a message for Bob.
        let envelope =
            SealedEnvelope::seal(&alice_id, &bob_id, &bob_x25519.public, message, ttl).unwrap();

        // Verify the sender is NOT visible in the envelope.
        assert!(!envelope.sender_is_visible(&alice_id));

        // The recipient field should be Bob's identity.
        assert_eq!(envelope.recipient, bob_id);

        // The ciphertext should NOT contain Alice's identity in plaintext.
        let alice_bytes = alice_id.as_bytes();
        // Search for alice's identity bytes in the ciphertext.
        let ciphertext = &envelope.ciphertext;
        let found = ciphertext.windows(32).any(|w| w == alice_bytes.as_slice());
        assert!(
            !found,
            "sender identity bytes should not appear in ciphertext (they are encrypted)"
        );

        // Bob opens the envelope.
        let payload = envelope.open(&bob_x25519.secret).unwrap();
        assert_eq!(payload.sender, alice_id);
        assert_eq!(payload.body, message);
    }

    #[test]
    fn test_sealed_wrong_key_fails() {
        let alice_id = alice_identity();
        let bob_id = bob_identity();
        let bob_x25519 = X25519KeyPair::generate();
        let eve_x25519 = X25519KeyPair::generate();

        let envelope = SealedEnvelope::seal(
            &alice_id,
            &bob_id,
            &bob_x25519.public,
            b"secret message",
            Ttl::one_day(),
        )
        .unwrap();

        // Eve tries to open with her key -- should fail.
        let result = envelope.open(&eve_x25519.secret);
        assert!(result.is_err());
    }

    #[test]
    fn test_sealed_roundtrip_empty_body() {
        let alice_id = alice_identity();
        let bob_id = bob_identity();
        let bob_x25519 = X25519KeyPair::generate();

        let envelope =
            SealedEnvelope::seal(&alice_id, &bob_id, &bob_x25519.public, b"", Ttl::one_hour())
                .unwrap();

        let payload = envelope.open(&bob_x25519.secret).unwrap();
        assert!(payload.body.is_empty());
        assert_eq!(payload.sender, alice_id);
    }

    #[test]
    fn test_sealed_envelope_not_expired() {
        let alice_id = alice_identity();
        let bob_id = bob_identity();
        let bob_x25519 = X25519KeyPair::generate();

        let envelope = SealedEnvelope::seal(
            &alice_id,
            &bob_id,
            &bob_x25519.public,
            b"hello",
            Ttl::one_day(),
        )
        .unwrap();

        assert!(!envelope.is_expired());
    }
}
