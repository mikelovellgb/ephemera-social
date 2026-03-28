//! Key export and import formats for multi-device support.
//!
//! Three formats: BIP39 mnemonic (24 words), QR code (37 bytes), and
//! encrypted backup (Argon2id + XChaCha20). All encode the 32-byte master
//! secret; recovering it reproduces the entire key hierarchy via HKDF.

use crate::keys::MasterSecret;
use ephemera_types::EphemeraError;
use zeroize::Zeroizing;

/// BIP39 English wordlist (2048 words). Each word maps to an 11-bit value.
/// We embed a minimal wordlist derived from the BIP39 specification.
const WORDLIST: &[&str; 2048] = &include!("bip39_words.txt");

/// QR binary format version byte.
const QR_VERSION: u8 = 0x01;

/// Length of the QR checksum suffix (first 4 bytes of BLAKE3 hash).
const QR_CHECKSUM_LEN: usize = 4;

/// Argon2id parameters for backup encryption (same as keystore).
const BACKUP_ARGON2_M_COST: u32 = 65_536;
const BACKUP_ARGON2_T_COST: u32 = 3;
const BACKUP_ARGON2_P_COST: u32 = 4;
const BACKUP_SALT_LEN: usize = 16;

/// Backup file format version.
const BACKUP_VERSION: [u8; 4] = [0x45, 0x50, 0x48, 0x01]; // "EPH" + version 1

/// Key export/import operations for multi-device identity transfer.
pub struct KeyExport;

impl KeyExport {
    /// Export the master secret as a BIP39 mnemonic (24 words for 256 bits).
    ///
    /// Encoding: 256 bits + 8-bit SHA-256 checksum = 264 bits = 24 x 11-bit words.
    #[must_use]
    pub fn to_mnemonic(master: &MasterSecret) -> Vec<String> {
        let bytes = master.as_bytes();

        // Compute checksum: first byte of SHA-256(master_secret)
        use sha2::{Digest, Sha256};
        let checksum_byte = {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            let hash = hasher.finalize();
            hash[0]
        };

        // Build 264-bit buffer: 256 bits of secret + 8 bits of checksum
        let mut bits = Vec::with_capacity(264);
        for &b in bytes.iter() {
            for i in (0..8).rev() {
                bits.push((b >> i) & 1);
            }
        }
        for i in (0..8).rev() {
            bits.push((checksum_byte >> i) & 1);
        }

        // Split into 24 groups of 11 bits, map to words
        let mut words = Vec::with_capacity(24);
        for chunk in bits.chunks(11) {
            let mut index: u16 = 0;
            for &bit in chunk {
                index = (index << 1) | u16::from(bit);
            }
            words.push(WORDLIST[index as usize].to_string());
        }

        words
    }

    /// Import a master secret from a BIP39 mnemonic.
    ///
    /// # Errors
    ///
    /// Returns an error if the word count is not 24, any word is not in the
    /// BIP39 wordlist, or the checksum does not match.
    pub fn from_mnemonic(words: &[String]) -> Result<MasterSecret, EphemeraError> {
        if words.len() != 24 {
            return Err(EphemeraError::InvalidKey {
                reason: format!("expected 24 mnemonic words, got {}", words.len()),
            });
        }

        // Convert words to 11-bit indices
        let mut bits = Vec::with_capacity(264);
        for word in words {
            let lower = word.to_lowercase();
            let index = WORDLIST
                .iter()
                .position(|&w| w == lower)
                .ok_or_else(|| EphemeraError::InvalidKey {
                    reason: format!("unknown mnemonic word: {word}"),
                })?;
            for i in (0..11).rev() {
                bits.push(((index >> i) & 1) as u8);
            }
        }

        // Extract 256 bits of secret + 8 bits of checksum
        let mut secret = Zeroizing::new([0u8; 32]);
        for (byte_idx, chunk) in bits[..256].chunks(8).enumerate() {
            let mut byte = 0u8;
            for &bit in chunk {
                byte = (byte << 1) | bit;
            }
            secret[byte_idx] = byte;
        }

        let mut checksum_byte = 0u8;
        for &bit in &bits[256..264] {
            checksum_byte = (checksum_byte << 1) | bit;
        }

        // Verify checksum
        use sha2::{Digest, Sha256};
        let expected_checksum = {
            let mut hasher = Sha256::new();
            hasher.update(secret.as_ref());
            let hash = hasher.finalize();
            hash[0]
        };

        if checksum_byte != expected_checksum {
            return Err(EphemeraError::InvalidKey {
                reason: "mnemonic checksum mismatch (typo in words?)".into(),
            });
        }

        Ok(MasterSecret::from_bytes(*secret))
    }

    /// Export as QR-encodable bytes: `version(1) || secret(32) || blake3_checksum(4)` = 37 bytes.
    #[must_use]
    pub fn to_qr_bytes(master: &MasterSecret) -> Vec<u8> {
        let mut data = Vec::with_capacity(1 + 32 + QR_CHECKSUM_LEN);
        data.push(QR_VERSION);
        data.extend_from_slice(master.as_bytes());

        let checksum = blake3::hash(&data);
        data.extend_from_slice(&checksum.as_bytes()[..QR_CHECKSUM_LEN]);

        data
    }

    /// Import from QR-encoded bytes. Validates version and checksum.
    pub fn from_qr_bytes(data: &[u8]) -> Result<MasterSecret, EphemeraError> {
        let expected_len = 1 + 32 + QR_CHECKSUM_LEN;
        if data.len() != expected_len {
            return Err(EphemeraError::InvalidKey {
                reason: format!(
                    "QR data wrong length: expected {expected_len}, got {}",
                    data.len()
                ),
            });
        }

        if data[0] != QR_VERSION {
            return Err(EphemeraError::InvalidKey {
                reason: format!("unsupported QR version: {:#04x}", data[0]),
            });
        }

        // Verify checksum: BLAKE3(version || master_secret)[..4]
        let payload = &data[..33]; // version + master_secret
        let stored_checksum = &data[33..37];
        let computed = blake3::hash(payload);
        let expected_checksum = &computed.as_bytes()[..QR_CHECKSUM_LEN];

        if stored_checksum != expected_checksum {
            return Err(EphemeraError::InvalidKey {
                reason: "QR checksum mismatch (data corrupted or tampered)".into(),
            });
        }

        let mut secret_bytes = Zeroizing::new([0u8; 32]);
        secret_bytes.copy_from_slice(&data[1..33]);
        Ok(MasterSecret::from_bytes(*secret_bytes))
    }

    /// Export the master secret as an encrypted backup file.
    ///
    /// The backup is encrypted with a user-provided passphrase via Argon2id
    /// key derivation and XChaCha20-Poly1305 AEAD encryption.
    ///
    /// Format: `version(4) || salt(16) || nonce+ciphertext+tag`
    ///
    /// # Errors
    ///
    /// Returns an error if encryption fails.
    pub fn to_encrypted_backup(
        master: &MasterSecret,
        passphrase: &str,
    ) -> Result<Vec<u8>, EphemeraError> {
        if passphrase.is_empty() {
            return Err(EphemeraError::InvalidKey {
                reason: "backup passphrase must not be empty".into(),
            });
        }

        let mut salt = [0u8; BACKUP_SALT_LEN];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut salt);

        let key = derive_backup_key(passphrase.as_bytes(), &salt)?;
        let sealed = crate::encryption::seal(&key, master.as_bytes())?;

        let mut output = Vec::with_capacity(4 + BACKUP_SALT_LEN + sealed.len());
        output.extend_from_slice(&BACKUP_VERSION);
        output.extend_from_slice(&salt);
        output.extend(sealed);

        Ok(output)
    }

    /// Import a master secret from an encrypted backup file.
    ///
    /// # Errors
    ///
    /// Returns an error if the passphrase is wrong, the data is corrupt,
    /// or the format version is unsupported.
    pub fn from_encrypted_backup(
        data: &[u8],
        passphrase: &str,
    ) -> Result<MasterSecret, EphemeraError> {
        let min_len = 4 + BACKUP_SALT_LEN + crate::encryption::NONCE_LEN
            + crate::encryption::TAG_LEN;
        if data.len() < min_len {
            return Err(EphemeraError::KeystoreError {
                reason: "backup file too short".into(),
            });
        }

        if data[..4] != BACKUP_VERSION {
            return Err(EphemeraError::KeystoreError {
                reason: "unsupported backup file version".into(),
            });
        }

        let mut salt = [0u8; BACKUP_SALT_LEN];
        salt.copy_from_slice(&data[4..4 + BACKUP_SALT_LEN]);

        let key = derive_backup_key(passphrase.as_bytes(), &salt)?;
        let sealed = &data[4 + BACKUP_SALT_LEN..];

        let plaintext = crate::encryption::open(&key, sealed).map_err(|_| {
            EphemeraError::KeystoreError {
                reason: "decryption failed (wrong passphrase or corrupt backup)".into(),
            }
        })?;

        if plaintext.len() != 32 {
            return Err(EphemeraError::InvalidKey {
                reason: format!(
                    "decrypted master secret wrong length: expected 32, got {}",
                    plaintext.len()
                ),
            });
        }

        let mut secret_bytes = Zeroizing::new([0u8; 32]);
        secret_bytes.copy_from_slice(&plaintext);
        Ok(MasterSecret::from_bytes(*secret_bytes))
    }
}

/// Derive an encryption key from a passphrase and salt using Argon2id.
fn derive_backup_key(
    passphrase: &[u8],
    salt: &[u8; BACKUP_SALT_LEN],
) -> Result<Zeroizing<[u8; 32]>, EphemeraError> {
    let argon2 = argon2::Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(
            BACKUP_ARGON2_M_COST,
            BACKUP_ARGON2_T_COST,
            BACKUP_ARGON2_P_COST,
            Some(32),
        )
        .map_err(|e| EphemeraError::KeyDerivationError {
            reason: format!("Argon2 params error: {e}"),
        })?,
    );

    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase, salt, &mut *key)
        .map_err(|e| EphemeraError::KeyDerivationError {
            reason: format!("Argon2id backup key derivation failed: {e}"),
        })?;
    Ok(key)
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
