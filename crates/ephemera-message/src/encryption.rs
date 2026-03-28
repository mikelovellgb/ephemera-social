//! Message encryption using X25519 + XChaCha20-Poly1305.
//!
//! This is the simplified PoC encryption layer. It performs a single
//! X25519 Diffie-Hellman exchange to derive a shared secret, then uses
//! HKDF to derive an encryption key for XChaCha20-Poly1305 AEAD.
//!
//! Phase 2 will replace this with the full Double Ratchet protocol for
//! forward secrecy and post-compromise security.

use ephemera_crypto::{
    encryption::{open, seal},
    x25519_diffie_hellman, X25519KeyPair, X25519PublicKey, X25519SecretKey,
};

use crate::MessageError;

/// HKDF info string for deriving message encryption keys.
const HKDF_INFO: &[u8] = b"ephemera-message-v1";

/// Minimum ciphertext length: 32 (ephemeral pubkey) + 24 (nonce) + 16 (tag).
const MIN_CIPHERTEXT_LEN: usize = 32 + 24 + 16;

/// Message encryption service.
///
/// Wraps the X25519 key exchange and symmetric encryption into a
/// simple encrypt/decrypt API for direct messages.
pub struct MessageEncryption;

impl MessageEncryption {
    /// Encrypt a plaintext message for a recipient.
    ///
    /// Performs an ephemeral X25519 key exchange with the recipient's
    /// public key, derives an encryption key via BLAKE3 keyed hash, and
    /// encrypts with XChaCha20-Poly1305.
    ///
    /// Returns `ephemeral_pubkey (32) || nonce (24) || ciphertext+tag`.
    pub fn encrypt_message(
        plaintext: &[u8],
        recipient_pubkey: &X25519PublicKey,
    ) -> Result<Vec<u8>, MessageError> {
        // Generate an ephemeral keypair for this message
        let ephemeral = X25519KeyPair::generate();

        // Compute shared secret via DH (returned in Zeroizing wrapper)
        let shared_secret = x25519_diffie_hellman(&ephemeral.secret, recipient_pubkey);

        // Derive encryption key
        let encryption_key = Self::derive_key(&shared_secret);

        // Encrypt with XChaCha20-Poly1305 (seal returns nonce || ciphertext)
        let sealed = seal(&encryption_key, plaintext).map_err(|e| {
            MessageError::Encryption(ephemera_crypto::CryptoError::EncryptionFailed {
                reason: e.to_string(),
            })
        })?;

        // Prepend the ephemeral public key so the recipient can derive
        // the shared secret
        let mut output = Vec::with_capacity(32 + sealed.len());
        output.extend_from_slice(ephemeral.public.as_bytes());
        output.extend_from_slice(&sealed);
        Ok(output)
    }

    /// Decrypt a message encrypted with [`MessageEncryption::encrypt_message`].
    ///
    /// The recipient uses their secret key to perform X25519 with the
    /// ephemeral public key embedded in the ciphertext, then derives
    /// the same encryption key and decrypts.
    pub fn decrypt_message(
        ciphertext: &[u8],
        our_secret: &X25519SecretKey,
    ) -> Result<Vec<u8>, MessageError> {
        if ciphertext.len() < MIN_CIPHERTEXT_LEN {
            return Err(MessageError::Encryption(
                ephemera_crypto::CryptoError::DecryptionFailed {
                    reason: "ciphertext too short".into(),
                },
            ));
        }

        // Extract the ephemeral public key (first 32 bytes)
        let mut ephemeral_bytes = [0u8; 32];
        ephemeral_bytes.copy_from_slice(&ciphertext[..32]);
        let ephemeral_pub = X25519PublicKey::from_bytes(ephemeral_bytes);

        // Compute the same shared secret (returned in Zeroizing wrapper)
        let shared_secret = x25519_diffie_hellman(our_secret, &ephemeral_pub);

        // Derive the same encryption key
        let encryption_key = Self::derive_key(&shared_secret);

        // Decrypt (the rest is nonce || ciphertext produced by seal)
        let plaintext = open(&encryption_key, &ciphertext[32..]).map_err(|e| {
            MessageError::Encryption(ephemera_crypto::CryptoError::DecryptionFailed {
                reason: e.to_string(),
            })
        })?;
        Ok(plaintext)
    }

    /// Derive a 32-byte encryption key from a DH shared secret.
    ///
    /// Uses BLAKE3 keyed hash as a KDF. In production the full Double
    /// Ratchet will use HKDF-SHA256 for the ratchet chain.
    fn derive_key(shared_secret: &[u8; 32]) -> [u8; 32] {
        let hash = blake3::keyed_hash(shared_secret, HKDF_INFO);
        *hash.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_round_trip() {
        let recipient = X25519KeyPair::generate();
        let plaintext = b"Hello, this is a secret message!";

        let encrypted = MessageEncryption::encrypt_message(plaintext, &recipient.public).unwrap();

        let decrypted = MessageEncryption::decrypt_message(&encrypted, &recipient.secret).unwrap();

        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let recipient = X25519KeyPair::generate();
        let wrong_key = X25519KeyPair::generate();
        let plaintext = b"secret";

        let encrypted = MessageEncryption::encrypt_message(plaintext, &recipient.public).unwrap();

        let result = MessageEncryption::decrypt_message(&encrypted, &wrong_key.secret);
        assert!(result.is_err());
    }

    #[test]
    fn empty_message_encrypts() {
        let recipient = X25519KeyPair::generate();
        let encrypted = MessageEncryption::encrypt_message(b"", &recipient.public).unwrap();
        let decrypted = MessageEncryption::decrypt_message(&encrypted, &recipient.secret).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn truncated_ciphertext_fails() {
        let result =
            MessageEncryption::decrypt_message(&[0u8; 10], &X25519KeyPair::generate().secret);
        assert!(result.is_err());
    }

    #[test]
    fn each_encryption_produces_different_output() {
        let recipient = X25519KeyPair::generate();
        let plaintext = b"same message";

        let enc1 = MessageEncryption::encrypt_message(plaintext, &recipient.public).unwrap();
        let enc2 = MessageEncryption::encrypt_message(plaintext, &recipient.public).unwrap();

        // Different ephemeral keys and nonces mean different ciphertext
        assert_ne!(enc1, enc2);
    }
}
