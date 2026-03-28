//! Identity types for content addressing and peer identification.
//!
//! Provides thin wrappers around raw byte arrays to give type safety
//! to the various identifier kinds used throughout Ephemera.

use serde::{Deserialize, Serialize};
use std::fmt;
use subtle::ConstantTimeEq;

/// Length of a BLAKE3 hash in bytes.
pub const BLAKE3_HASH_LEN: usize = 32;

/// Version byte prepended to content hashes on the wire.
pub const CONTENT_HASH_VERSION: u8 = 0x01;

/// Length of an Ed25519 public key in bytes.
pub const ED25519_PUBKEY_LEN: usize = 32;

/// Content identifier derived from a BLAKE3 hash of the content bytes.
///
/// The wire format is `[version_byte || 32-byte hash]` (33 bytes total),
/// but in memory we store version and hash separately for convenience.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentId {
    version: u8,
    hash: [u8; BLAKE3_HASH_LEN],
}

impl ContentId {
    /// Create a `ContentId` from a raw 32-byte BLAKE3 hash.
    #[must_use]
    pub fn from_hash(hash: [u8; BLAKE3_HASH_LEN]) -> Self {
        Self {
            version: CONTENT_HASH_VERSION,
            hash,
        }
    }

    /// Create a `ContentId` from a raw 32-byte digest.
    ///
    /// Alias for [`from_hash`](Self::from_hash) used by crates that refer
    /// to content identifiers as "content hashes".
    #[must_use]
    pub fn from_digest(digest: [u8; BLAKE3_HASH_LEN]) -> Self {
        Self::from_hash(digest)
    }

    /// Create a `ContentId` from a 33-byte wire representation.
    ///
    /// Returns `None` if the slice length is wrong or the version byte
    /// is unrecognized.
    #[must_use]
    pub fn from_wire_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != BLAKE3_HASH_LEN + 1 {
            return None;
        }
        if bytes[0] != CONTENT_HASH_VERSION {
            return None;
        }
        let mut hash = [0u8; BLAKE3_HASH_LEN];
        hash.copy_from_slice(&bytes[1..]);
        Some(Self {
            version: bytes[0],
            hash,
        })
    }

    /// Encode to the 33-byte wire format.
    #[must_use]
    pub fn to_wire_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(BLAKE3_HASH_LEN + 1);
        out.push(self.version);
        out.extend_from_slice(&self.hash);
        out
    }

    /// The raw 32-byte BLAKE3 hash.
    #[must_use]
    pub fn hash_bytes(&self) -> &[u8; BLAKE3_HASH_LEN] {
        &self.hash
    }

    /// The version byte.
    #[must_use]
    pub fn version(&self) -> u8 {
        self.version
    }

    /// Constant-time equality comparison of wire bytes.
    ///
    /// Prevents timing side-channels when comparing content hashes
    /// (e.g., during content integrity verification).
    #[must_use]
    pub fn wire_bytes_ct_eq(&self, other: &Self) -> bool {
        let version_eq = self.version.ct_eq(&other.version);
        let hash_eq = self.hash.ct_eq(&other.hash);
        (version_eq & hash_eq).unwrap_u8() == 1
    }
}

impl fmt::Debug for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentId(v{}:{})", self.version, hex::encode(self.hash))
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{IdentityKey, NodeId};

    #[test]
    fn content_id_round_trip() {
        let hash = [0xABu8; BLAKE3_HASH_LEN];
        let id = ContentId::from_hash(hash);
        let wire = id.to_wire_bytes();
        assert_eq!(wire.len(), 33);
        assert_eq!(wire[0], CONTENT_HASH_VERSION);

        let recovered = ContentId::from_wire_bytes(&wire).expect("valid wire bytes");
        assert_eq!(recovered, id);
    }

    #[test]
    fn content_id_rejects_bad_version() {
        let mut wire = vec![0xFF];
        wire.extend_from_slice(&[0u8; BLAKE3_HASH_LEN]);
        assert!(ContentId::from_wire_bytes(&wire).is_none());
    }

    #[test]
    fn content_id_rejects_bad_length() {
        assert!(ContentId::from_wire_bytes(&[0x01, 0x02]).is_none());
    }

    #[test]
    fn node_id_display_is_hex() {
        let id = NodeId::from_bytes([0x01; 32]);
        let display = format!("{id}");
        assert_eq!(display.len(), 64);
    }

    #[test]
    fn identity_key_to_hex_via_id() {
        let id = IdentityKey::from_bytes([0xCC; 32]);
        assert_eq!(id.to_hex(), hex::encode([0xCC; 32]));
    }
}
