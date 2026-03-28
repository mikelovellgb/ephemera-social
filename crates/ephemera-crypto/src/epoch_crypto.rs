//! Epoch-keyed encryption and decryption helpers.
//!
//! Extends [`EpochKeyManager`] with methods that derive the correct epoch
//! key and then encrypt or decrypt content using XChaCha20-Poly1305.

use super::*;
use crate::encryption;

impl EpochKeyManager {
    /// Encrypt plaintext with the current epoch key.
    /// Returns `(epoch_id, sealed_ciphertext)`.
    pub fn encrypt_with_current_epoch(
        &mut self,
        plaintext: &[u8],
    ) -> Result<(u64, Vec<u8>), ephemera_types::EphemeraError> {
        let (epoch_id, key) = self.current_epoch_key()?;
        let sealed = encryption::seal(&key, plaintext)?;
        Ok((epoch_id, sealed))
    }

    /// Decrypt ciphertext using the epoch key for the given epoch.
    /// Returns `None` if the key has been destroyed (cryptographic shredding).
    pub fn decrypt_with_epoch_key(
        &mut self,
        epoch_id: u64,
        sealed: &[u8],
    ) -> Result<Option<Vec<u8>>, ephemera_types::EphemeraError> {
        let key = match self.epoch_key_for(epoch_id)? {
            Some(k) => k,
            None => return Ok(None),
        };
        let plaintext = encryption::open(&key, sealed)?;
        Ok(Some(plaintext))
    }
}

impl std::fmt::Debug for EpochKeyManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochKeyManager")
            .field("active_keys", &self.keys.len())
            .field("destroyed_epochs", &self.destroyed_epochs.len())
            .finish()
    }
}
