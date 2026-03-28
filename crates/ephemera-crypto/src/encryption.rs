//! Symmetric encryption using XChaCha20-Poly1305.
//!
//! XChaCha20-Poly1305 is chosen for its 192-bit nonce (safe for random
//! generation without birthday-bound concerns) and its AEAD properties.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use ephemera_types::EphemeraError;
use rand::rngs::OsRng;
use rand::RngCore;

/// Nonce length for XChaCha20-Poly1305 (24 bytes).
pub const NONCE_LEN: usize = 24;

/// Key length for XChaCha20-Poly1305 (32 bytes).
pub const KEY_LEN: usize = 32;

/// Poly1305 authentication tag length (16 bytes).
pub const TAG_LEN: usize = 16;

/// Generate a random 24-byte nonce suitable for XChaCha20-Poly1305.
///
/// The 192-bit nonce space is large enough that random generation
/// is safe without a counter, even for very high message volumes.
#[must_use]
pub fn generate_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Encrypt plaintext with XChaCha20-Poly1305.
///
/// Returns the ciphertext (which includes the 16-byte Poly1305 tag
/// appended by the AEAD implementation).
///
/// # Arguments
///
/// * `key` - 32-byte symmetric key.
/// * `nonce` - 24-byte nonce (must be unique per key; use `generate_nonce`).
/// * `plaintext` - The data to encrypt.
///
/// # Errors
///
/// Returns `EphemeraError::EncryptionError` if encryption fails.
pub fn encrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>, EphemeraError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let xnonce = XNonce::from_slice(nonce);
    cipher
        .encrypt(xnonce, plaintext)
        .map_err(|e| EphemeraError::EncryptionError {
            reason: format!("XChaCha20-Poly1305 encryption failed: {e}"),
        })
}

/// Decrypt ciphertext with XChaCha20-Poly1305.
///
/// The ciphertext must include the 16-byte Poly1305 authentication tag
/// (as produced by `encrypt`).
///
/// # Errors
///
/// Returns `EphemeraError::EncryptionError` if decryption or authentication fails.
pub fn decrypt(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, EphemeraError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let xnonce = XNonce::from_slice(nonce);
    cipher
        .decrypt(xnonce, ciphertext)
        .map_err(|e| EphemeraError::EncryptionError {
            reason: format!("XChaCha20-Poly1305 decryption failed: {e}"),
        })
}

/// Encrypt plaintext and prepend the nonce to the output.
///
/// Returns nonce + ciphertext + tag concatenated.
/// This is a convenience wrapper that generates a random nonce and
/// packages it with the ciphertext for storage or transmission.
///
/// # Errors
///
/// Returns an error if encryption fails.
pub fn seal(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<Vec<u8>, EphemeraError> {
    let nonce = generate_nonce();
    let ciphertext = encrypt(key, &nonce, plaintext)?;
    let mut output = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    output.extend_from_slice(&nonce);
    output.extend(ciphertext);
    Ok(output)
}

/// Decrypt a sealed message (nonce prepended to ciphertext).
///
/// Expects the format produced by `seal`: nonce + ciphertext + tag.
///
/// # Errors
///
/// Returns an error if the input is too short or decryption fails.
pub fn open(key: &[u8; KEY_LEN], sealed: &[u8]) -> Result<Vec<u8>, EphemeraError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(EphemeraError::EncryptionError {
            reason: format!(
                "sealed message too short: {} bytes (minimum {})",
                sealed.len(),
                NONCE_LEN + TAG_LEN
            ),
        });
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&sealed[..NONCE_LEN]);
    let ciphertext = &sealed[NONCE_LEN..];
    decrypt(key, &nonce, ciphertext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; KEY_LEN] {
        let mut key = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut key);
        key
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = test_key();
        let nonce = generate_nonce();
        let plaintext = b"hello ephemera, this is a secret message";
        let ciphertext = encrypt(&key, &nonce, plaintext).unwrap();
        let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = test_key();
        let key2 = test_key();
        let nonce = generate_nonce();
        let ciphertext = encrypt(&key1, &nonce, b"secret").unwrap();
        assert!(decrypt(&key2, &nonce, &ciphertext).is_err());
    }

    #[test]
    fn wrong_nonce_fails() {
        let key = test_key();
        let nonce1 = generate_nonce();
        let nonce2 = generate_nonce();
        let ciphertext = encrypt(&key, &nonce1, b"secret").unwrap();
        assert!(decrypt(&key, &nonce2, &ciphertext).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = test_key();
        let nonce = generate_nonce();
        let mut ciphertext = encrypt(&key, &nonce, b"secret").unwrap();
        if let Some(byte) = ciphertext.last_mut() {
            *byte ^= 0xFF;
        }
        assert!(decrypt(&key, &nonce, &ciphertext).is_err());
    }

    #[test]
    fn seal_open_round_trip() {
        let key = test_key();
        let plaintext = b"sealed secret message for storage";
        let sealed = seal(&key, plaintext).unwrap();
        assert_eq!(sealed.len(), NONCE_LEN + plaintext.len() + TAG_LEN);
        let opened = open(&key, &sealed).unwrap();
        assert_eq!(&opened, plaintext);
    }

    #[test]
    fn open_rejects_too_short() {
        let key = test_key();
        let result = open(&key, &[0u8; 10]);
        assert!(result.is_err());
    }

    #[test]
    fn empty_plaintext() {
        let key = test_key();
        let sealed = seal(&key, b"").unwrap();
        let opened = open(&key, &sealed).unwrap();
        assert!(opened.is_empty());
    }
}

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Encrypt then decrypt with the same key always recovers the plaintext.
        #[test]
        fn prop_encrypt_decrypt_round_trip(plaintext in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let key = test_key();
            let nonce = generate_nonce();
            let ciphertext = encrypt(&key, &nonce, &plaintext).unwrap();
            let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();
            prop_assert_eq!(decrypted, plaintext);
        }

        /// Seal then open with the same key always recovers the plaintext.
        #[test]
        fn prop_seal_open_round_trip(plaintext in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let key = test_key();
            let sealed = seal(&key, &plaintext).unwrap();
            let opened = open(&key, &sealed).unwrap();
            prop_assert_eq!(opened, plaintext);
        }

        /// Sealed output length is always nonce + plaintext + tag.
        #[test]
        fn prop_sealed_length(plaintext in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let key = test_key();
            let sealed = seal(&key, &plaintext).unwrap();
            prop_assert_eq!(sealed.len(), NONCE_LEN + plaintext.len() + TAG_LEN);
        }

        /// Any single-byte corruption in sealed data causes decryption to fail.
        #[test]
        fn prop_tamper_detection(
            plaintext in proptest::collection::vec(any::<u8>(), 1..256),
            flip_pos_pct in 0usize..100,
        ) {
            let key = test_key();
            let mut sealed = seal(&key, &plaintext).unwrap();
            // Flip a byte at a position proportional to the sealed length.
            let pos = flip_pos_pct * (sealed.len() - 1) / 99;
            sealed[pos] ^= 0x01;
            prop_assert!(open(&key, &sealed).is_err());
        }
    }

    fn test_key() -> [u8; KEY_LEN] {
        let mut key = [0u8; KEY_LEN];
        OsRng.fill_bytes(&mut key);
        key
    }
}
