//! Encrypted keystore for persisting key material to disk.
//!
//! The keystore format is:
//! `[4-byte version][16-byte Argon2id salt][24-byte nonce][ciphertext][16-byte Poly1305 tag]`
//!
//! The encryption key is derived from a user passphrase via Argon2id.

use crate::encryption;
use argon2::Argon2;
use ephemera_types::EphemeraError;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Current keystore file format version.
const KEYSTORE_VERSION: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// Length of the Argon2id salt.
const SALT_LEN: usize = 16;

/// Argon2id parameters for key derivation.
const ARGON2_M_COST: u32 = 65_536; // 64 MiB in KiB
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;

/// The serializable contents of a keystore (everything that gets encrypted).
///
/// Implements [`ZeroizeOnDrop`] to wipe all secret material on drop.
/// The [`Debug`] implementation redacts secret fields. `Clone` is
/// intentionally NOT derived to prevent uncontrolled copies of secrets.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct KeystoreContents {
    /// The master secret bytes (32 bytes).
    pub master_secret: [u8; 32],
    /// Node identity secret key bytes (32 bytes).
    pub node_secret: [u8; 32],
    /// Pseudonym key indices and their secret bytes.
    pub pseudonym_secrets: Vec<PseudonymEntry>,
}

impl std::fmt::Debug for KeystoreContents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeystoreContents")
            .field("master_secret", &"[REDACTED]")
            .field("node_secret", &"[REDACTED]")
            .field(
                "pseudonym_secrets",
                &format_args!("[{} entries, REDACTED]", self.pseudonym_secrets.len()),
            )
            .finish()
    }
}

/// A single pseudonym entry in the keystore.
///
/// Secret material is zeroized on drop. The [`Debug`] implementation
/// redacts the secret bytes.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct PseudonymEntry {
    /// Derivation index.
    pub index: u32,
    /// The 32-byte secret key.
    pub secret: [u8; 32],
}

impl std::fmt::Debug for PseudonymEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PseudonymEntry")
            .field("index", &self.index)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

/// Derive an encryption key from a passphrase and salt using Argon2id.
///
/// Returns the key wrapped in [`Zeroizing`] so it is wiped on drop.
fn derive_key_from_passphrase(
    passphrase: &[u8],
    salt: &[u8; SALT_LEN],
) -> Result<Zeroizing<[u8; 32]>, EphemeraError> {
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST, Some(32)).map_err(
            |e| EphemeraError::KeyDerivationError {
                reason: format!("Argon2 params error: {e}"),
            },
        )?,
    );

    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase, salt, &mut *key)
        .map_err(|e| EphemeraError::KeyDerivationError {
            reason: format!("Argon2id key derivation failed: {e}"),
        })?;
    Ok(key)
}

/// Save keystore contents to a file, encrypted with the given passphrase.
///
/// # Errors
///
/// Returns an error if serialization, encryption, or file I/O fails.
pub fn save_keystore(
    path: &Path,
    passphrase: &[u8],
    contents: &KeystoreContents,
) -> Result<(), EphemeraError> {
    let plaintext =
        serde_json::to_vec(contents).map_err(|e| EphemeraError::SerializationError {
            reason: format!("keystore serialization failed: {e}"),
        })?;

    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let key = derive_key_from_passphrase(passphrase, &salt)?;

    let sealed = encryption::seal(&key, &plaintext)?;

    let mut file_contents = Vec::with_capacity(4 + SALT_LEN + sealed.len());
    file_contents.extend_from_slice(&KEYSTORE_VERSION);
    file_contents.extend_from_slice(&salt);
    file_contents.extend(sealed);

    std::fs::write(path, &file_contents)?;
    Ok(())
}

/// Load and decrypt keystore contents from a file.
///
/// # Errors
///
/// Returns an error if the file cannot be read, the passphrase is wrong,
/// or the data is corrupt.
pub fn load_keystore(path: &Path, passphrase: &[u8]) -> Result<KeystoreContents, EphemeraError> {
    let file_contents = std::fs::read(path)?;

    let min_len = 4 + SALT_LEN + encryption::NONCE_LEN + encryption::TAG_LEN;
    if file_contents.len() < min_len {
        return Err(EphemeraError::KeystoreError {
            reason: "keystore file too short".into(),
        });
    }

    if file_contents[..4] != KEYSTORE_VERSION {
        return Err(EphemeraError::KeystoreError {
            reason: "unsupported keystore version".into(),
        });
    }

    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&file_contents[4..4 + SALT_LEN]);

    let key = derive_key_from_passphrase(passphrase, &salt)?;

    let sealed = &file_contents[4 + SALT_LEN..];
    let plaintext = encryption::open(&key, sealed).map_err(|_| EphemeraError::KeystoreError {
        reason: "decryption failed (wrong passphrase or corrupt keystore)".into(),
    })?;

    let contents: KeystoreContents =
        serde_json::from_slice(&plaintext).map_err(|e| EphemeraError::KeystoreError {
            reason: format!("keystore deserialization failed: {e}"),
        })?;

    Ok(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystore_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.keystore");

        let contents = KeystoreContents {
            master_secret: [0xAA; 32],
            node_secret: [0xBB; 32],
            pseudonym_secrets: vec![
                PseudonymEntry {
                    index: 0,
                    secret: [0xCC; 32],
                },
                PseudonymEntry {
                    index: 1,
                    secret: [0xDD; 32],
                },
            ],
        };

        save_keystore(&path, b"test-passphrase", &contents).unwrap();
        let loaded = load_keystore(&path, b"test-passphrase").unwrap();

        assert_eq!(loaded.master_secret, contents.master_secret);
        assert_eq!(loaded.node_secret, contents.node_secret);
        assert_eq!(loaded.pseudonym_secrets.len(), 2);
        assert_eq!(loaded.pseudonym_secrets[0].secret, [0xCC; 32]);
    }

    #[test]
    fn keystore_wrong_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.keystore");

        let contents = KeystoreContents {
            master_secret: [0xAA; 32],
            node_secret: [0xBB; 32],
            pseudonym_secrets: vec![],
        };

        save_keystore(&path, b"correct-passphrase", &contents).unwrap();
        let result = load_keystore(&path, b"wrong-passphrase");
        assert!(result.is_err());
    }

    #[test]
    fn keystore_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.keystore");

        std::fs::write(&path, b"too short").unwrap();
        let result = load_keystore(&path, b"passphrase");
        assert!(result.is_err());
    }

    #[test]
    fn keystore_bad_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("badver.keystore");

        let mut data = vec![0xFF, 0xFF, 0xFF, 0xFF]; // bad version
        data.extend_from_slice(&[0u8; SALT_LEN]);
        data.extend_from_slice(&[0u8; encryption::NONCE_LEN + encryption::TAG_LEN + 10]);
        std::fs::write(&path, &data).unwrap();

        let result = load_keystore(&path, b"passphrase");
        assert!(result.is_err());
    }

    #[test]
    fn debug_redacts_secrets() {
        let contents = KeystoreContents {
            master_secret: [0xAA; 32],
            node_secret: [0xBB; 32],
            pseudonym_secrets: vec![PseudonymEntry {
                index: 0,
                secret: [0xCC; 32],
            }],
        };

        let dbg = format!("{contents:?}");
        assert!(dbg.contains("REDACTED"), "Debug must redact secrets");
        // Must NOT contain the raw hex of any secret byte.
        assert!(
            !dbg.contains("aa") && !dbg.contains("AA"),
            "Debug must not leak master_secret bytes"
        );
        assert!(
            !dbg.contains("bb") && !dbg.contains("BB"),
            "Debug must not leak node_secret bytes"
        );
    }
}
