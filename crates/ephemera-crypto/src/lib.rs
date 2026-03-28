//! Cryptographic operations for the Ephemera decentralized social platform.
//!
//! This crate provides:
//! - Ed25519 signing and verification (identity, content signatures)
//! - XChaCha20-Poly1305 symmetric encryption (content, keystore)
//! - BLAKE3 hashing (content addressing)
//! - HKDF-SHA256 key derivation (pseudonyms, epochs)
//! - Argon2id passphrase-based key derivation (keystore)
//! - Proof-of-work (spam deterrence)
//! - Encrypted keystore (persistent key storage)

pub mod device;
pub mod dh;
pub mod encryption;
pub mod epoch;
mod error;
pub mod export;
pub mod hash;
pub mod identity;
pub mod keys;
pub mod keystore;
pub mod pow;
pub mod signing;

// Re-export commonly used items.
pub use device::{DeviceInfo, DeviceManager, Platform};
pub use dh::{x25519_diffie_hellman, X25519KeyPair, X25519PublicKey, X25519SecretKey};
pub use encryption::{decrypt, encrypt, open, seal};
pub use epoch::{
    current_epoch_id, epoch_id_for_timestamp, DestructionResult, EpochKeyManager, RotationResult,
    EPOCH_DURATION_SECS, MAX_EPOCH_KEY_AGE_SECS,
};
pub use error::CryptoError;
pub use export::KeyExport;
pub use hash::{blake3_hash, content_id_from_bytes, verify_content_id, IncrementalHasher};
pub use identity::verify_signature as verify_signature_bytes;
pub use identity::{NodeIdentity, PseudonymIdentity};
pub use keys::{
    derive_epoch_key, derive_pseudonym_key, ed25519_pub_to_x25519, ed25519_seed_to_x25519,
    hkdf_derive, KeyPair, MasterSecret, PublicKeyBytes,
};
pub use keystore::{load_keystore, save_keystore, KeystoreContents, PseudonymEntry};
pub use pow::{generate_pow, verify_pow, PowStamp};
pub use signing::{verify_signature, SigningKeyPair};

/// Encrypt plaintext with XChaCha20-Poly1305, returning `nonce || ciphertext`.
///
/// Convenience alias for [`seal`] that returns a `CryptoError` on failure.
pub fn encrypt_xchacha20(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    seal(key, plaintext).map_err(|e| CryptoError::EncryptionFailed {
        reason: e.to_string(),
    })
}

/// Decrypt ciphertext produced by [`encrypt_xchacha20`].
///
/// Expects `nonce || ciphertext` format.
pub fn decrypt_xchacha20(key: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    open(key, sealed).map_err(|e| CryptoError::DecryptionFailed {
        reason: e.to_string(),
    })
}
