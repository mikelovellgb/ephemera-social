//! Epoch key management for cryptographic shredding.
//!
//! Keys rotate on a 24-hour schedule. After 30 days, epoch keys are destroyed,
//! making content permanently undecryptable.

use crate::keys::{derive_epoch_key, MasterSecret};
use std::collections::HashMap;
use zeroize::Zeroize;

/// Duration of one epoch in seconds (24 hours).
pub const EPOCH_DURATION_SECS: u64 = 24 * 3_600;

/// Maximum age of an epoch key before it must be destroyed (30 days in seconds).
pub const MAX_EPOCH_KEY_AGE_SECS: u64 = 30 * EPOCH_DURATION_SECS;

/// Compute the epoch ID for a given Unix timestamp.
///
/// The epoch ID is simply the day number: `timestamp / 86400`.
#[must_use]
pub fn epoch_id_for_timestamp(unix_secs: u64) -> u64 {
    unix_secs / EPOCH_DURATION_SECS
}

/// Compute the current epoch ID based on the system clock.
#[must_use]
pub fn current_epoch_id() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs();
    epoch_id_for_timestamp(now)
}

/// An epoch key entry with metadata about when it was created and whether
/// it has been destroyed.
#[derive(Debug, Clone)]
struct EpochKeyEntry {
    /// The 32-byte symmetric key for this epoch.
    key: [u8; 32],
    /// The epoch number (day number). Retained for diagnostics and logging.
    #[allow(dead_code)]
    epoch_id: u64,
    /// Unix timestamp when this entry was created. Retained for diagnostics.
    #[allow(dead_code)]
    created_at: u64,
    /// Whether this key has been destroyed (set to true on destruction).
    destroyed: bool,
}

impl Drop for EpochKeyEntry {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

/// Result of an epoch key rotation.
#[derive(Debug)]
pub struct RotationResult {
    /// The new epoch ID that was activated.
    pub new_epoch_id: u64,
    /// Epoch IDs that are now marked for future destruction.
    pub marked_for_destruction: Vec<u64>,
}

/// Result of destroying expired epoch keys.
#[derive(Debug)]
pub struct DestructionResult {
    /// Epoch IDs whose keys were destroyed.
    pub destroyed_epoch_ids: Vec<u64>,
    /// Number of keys destroyed.
    pub count: usize,
}

/// Manages epoch keys for the cryptographic shredding lifecycle.
///
/// Keys are derived deterministically from a master secret using HKDF,
/// so they can be re-derived as long as the master secret exists. However,
/// once the manager marks a key as "destroyed", it is zeroed from memory
/// and will not be re-derived.
///
/// # Usage
pub struct EpochKeyManager {
    /// The master secret from which epoch keys are derived.
    master: MasterSecret,
    /// Cache of epoch key entries (epoch_id -> entry).
    keys: HashMap<u64, EpochKeyEntry>,
    /// Set of epoch IDs that have been permanently destroyed.
    /// We track these so we never re-derive a destroyed key.
    destroyed_epochs: Vec<u64>,
}

impl EpochKeyManager {
    /// Create a new epoch key manager with the given master secret.
    ///
    /// The manager starts with no cached keys. Keys are derived on demand
    /// via `current_epoch_key()` or `epoch_key_for()`.
    #[must_use]
    pub fn new(master: MasterSecret) -> Self {
        Self {
            master,
            keys: HashMap::new(),
            destroyed_epochs: Vec::new(),
        }
    }

    /// Return the current epoch's key, deriving it if necessary.
    ///
    /// Returns `(epoch_id, key)` where `epoch_id` is today's day number.
    ///
    /// # Errors
    ///
    /// Returns an error if key derivation fails (should not happen with valid parameters).
    pub fn current_epoch_key(&mut self) -> Result<(u64, [u8; 32]), ephemera_types::EphemeraError> {
        let epoch_id = current_epoch_id();
        let key = self.ensure_key(epoch_id)?;
        Ok((epoch_id, key))
    }

    /// Return the key for a specific epoch, if it is still retained.
    ///
    /// Returns `None` if the key has been destroyed (cryptographic shredding
    /// has occurred). Returns `Some(key)` if the key is still available.
    ///
    /// # Errors
    ///
    /// Returns an error if key derivation fails.
    pub fn epoch_key_for(
        &mut self,
        epoch_id: u64,
    ) -> Result<Option<[u8; 32]>, ephemera_types::EphemeraError> {
        if self.destroyed_epochs.contains(&epoch_id) {
            return Ok(None);
        }
        let key = self.ensure_key(epoch_id)?;
        Ok(Some(key))
    }

    /// Rotate keys: ensure the current epoch's key exists and mark old
    /// epochs for future destruction.
    ///
    /// This does NOT immediately destroy keys -- it just ensures the
    /// current key is derived and identifies which old keys are candidates
    /// for destruction.
    ///
    /// # Errors
    ///
    /// Returns an error if key derivation fails.
    pub fn rotate(&mut self) -> Result<RotationResult, ephemera_types::EphemeraError> {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs();
        let current = epoch_id_for_timestamp(now_secs);

        // Ensure current epoch key exists.
        self.ensure_key(current)?;

        // Find epochs older than 30 days that should be marked for destruction.
        let cutoff_epoch = current.saturating_sub(MAX_EPOCH_KEY_AGE_SECS / EPOCH_DURATION_SECS);

        let marked: Vec<u64> = self
            .keys
            .keys()
            .filter(|&&eid| eid < cutoff_epoch && !self.destroyed_epochs.contains(&eid))
            .copied()
            .collect();

        Ok(RotationResult {
            new_epoch_id: current,
            marked_for_destruction: marked,
        })
    }

    /// Destroy all epoch keys older than 30 days.
    ///
    /// Once a key is destroyed:
    /// - It is zeroed from memory.
    /// - Its epoch ID is added to the destroyed list.
    /// - It will never be re-derived.
    /// - Any content encrypted under that key is PERMANENTLY undecryptable.
    ///
    /// This is the cryptographic shredding operation.
    pub fn destroy_expired_keys(&mut self) -> DestructionResult {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs();
        self.destroy_expired_keys_at(now_secs)
    }

    /// Destroy all epoch keys that would be expired at the given timestamp.
    ///
    /// This is the testable version of `destroy_expired_keys` that accepts
    /// an explicit "now" timestamp.
    pub fn destroy_expired_keys_at(&mut self, now_secs: u64) -> DestructionResult {
        let current_epoch = epoch_id_for_timestamp(now_secs);
        let cutoff_epoch = if current_epoch > (MAX_EPOCH_KEY_AGE_SECS / EPOCH_DURATION_SECS) {
            current_epoch - (MAX_EPOCH_KEY_AGE_SECS / EPOCH_DURATION_SECS)
        } else {
            return DestructionResult {
                destroyed_epoch_ids: Vec::new(),
                count: 0,
            };
        };

        let to_destroy: Vec<u64> = self
            .keys
            .keys()
            .filter(|&&eid| eid < cutoff_epoch)
            .copied()
            .collect();

        let count = to_destroy.len();

        for &eid in &to_destroy {
            // Remove and drop the entry (zeroize on drop).
            self.keys.remove(&eid);
            if !self.destroyed_epochs.contains(&eid) {
                self.destroyed_epochs.push(eid);
            }
        }

        DestructionResult {
            destroyed_epoch_ids: to_destroy,
            count,
        }
    }

    /// Manually destroy the key for a specific epoch.
    ///
    /// Returns `true` if the key was present and destroyed, `false` if it
    /// was already destroyed or never existed.
    pub fn destroy_key(&mut self, epoch_id: u64) -> bool {
        let removed = self.keys.remove(&epoch_id).is_some();
        if !self.destroyed_epochs.contains(&epoch_id) {
            self.destroyed_epochs.push(epoch_id);
        }
        removed
    }

    /// Check whether a given epoch's key has been destroyed.
    #[must_use]
    pub fn is_destroyed(&self, epoch_id: u64) -> bool {
        self.destroyed_epochs.contains(&epoch_id)
    }

    /// Return the list of all epoch IDs whose keys have been destroyed.
    #[must_use]
    pub fn destroyed_epoch_ids(&self) -> &[u64] {
        &self.destroyed_epochs
    }

    /// Return the number of active (non-destroyed) keys in the cache.
    #[must_use]
    pub fn active_key_count(&self) -> usize {
        self.keys.len()
    }

    fn ensure_key(&mut self, epoch_id: u64) -> Result<[u8; 32], ephemera_types::EphemeraError> {
        if let Some(entry) = self.keys.get(&epoch_id) {
            if entry.destroyed {
                return Err(ephemera_types::EphemeraError::EncryptionError {
                    reason: format!("epoch key {epoch_id} has been destroyed"),
                });
            }
            return Ok(entry.key);
        }
        if self.destroyed_epochs.contains(&epoch_id) {
            return Err(ephemera_types::EphemeraError::EncryptionError {
                reason: format!("epoch key {epoch_id} has been destroyed"),
            });
        }
        let key = derive_epoch_key(self.master.as_bytes(), epoch_id)?;
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_secs();
        self.keys.insert(
            epoch_id,
            EpochKeyEntry {
                key,
                epoch_id,
                created_at: now_secs,
                destroyed: false,
            },
        );
        Ok(key)
    }
}

#[path = "epoch_crypto.rs"]
mod epoch_crypto;

#[cfg(test)]
#[path = "epoch_tests.rs"]
mod tests;
