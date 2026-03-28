//! BLAKE3 hashing utilities.
//!
//! Provides content-addressed hashing for the Ephemera network.
//! All content identifiers are derived from BLAKE3 hashes of the
//! content bytes.

use ephemera_types::ContentId;

/// Hash arbitrary bytes with BLAKE3 and return a [`ContentId`].
///
/// This is the canonical way to compute content identifiers for posts,
/// media chunks, and any other content-addressed data.
#[must_use]
pub fn content_id_from_bytes(data: &[u8]) -> ContentId {
    let hash = blake3::hash(data);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(hash.as_bytes());
    ContentId::from_hash(arr)
}

/// Hash arbitrary bytes with BLAKE3 and return the raw 32-byte digest.
#[must_use]
pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    let hash = blake3::hash(data);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(hash.as_bytes());
    arr
}

/// Incrementally hash a stream of byte slices with BLAKE3.
///
/// Useful for large files where the entire content may not fit in memory.
pub struct IncrementalHasher {
    hasher: blake3::Hasher,
}

impl IncrementalHasher {
    /// Create a new incremental hasher.
    #[must_use]
    pub fn new() -> Self {
        Self {
            hasher: blake3::Hasher::new(),
        }
    }

    /// Feed more bytes into the hasher.
    pub fn update(&mut self, data: &[u8]) {
        self.hasher.update(data);
    }

    /// Finalize the hash and return a [`ContentId`].
    #[must_use]
    pub fn finalize_content_id(self) -> ContentId {
        let hash = self.hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(hash.as_bytes());
        ContentId::from_hash(arr)
    }

    /// Finalize the hash and return the raw 32-byte digest.
    #[must_use]
    pub fn finalize_bytes(self) -> [u8; 32] {
        let hash = self.hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(hash.as_bytes());
        arr
    }
}

impl Default for IncrementalHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Verify that a byte slice matches an expected [`ContentId`].
///
/// Uses constant-time comparison to prevent timing side-channels.
#[must_use]
pub fn verify_content_id(data: &[u8], expected: &ContentId) -> bool {
    let actual = content_id_from_bytes(data);
    actual.wire_bytes_ct_eq(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_deterministic() {
        let data = b"hello ephemera";
        let id1 = content_id_from_bytes(data);
        let id2 = content_id_from_bytes(data);
        assert_eq!(id1, id2);
    }

    #[test]
    fn different_data_different_hash() {
        let id1 = content_id_from_bytes(b"hello");
        let id2 = content_id_from_bytes(b"world");
        assert_ne!(id1, id2);
    }

    #[test]
    fn incremental_matches_oneshot() {
        let data = b"hello ephemera world";
        let oneshot = content_id_from_bytes(data);

        let mut inc = IncrementalHasher::new();
        inc.update(b"hello ");
        inc.update(b"ephemera ");
        inc.update(b"world");
        let incremental = inc.finalize_content_id();

        assert_eq!(oneshot, incremental);
    }

    #[test]
    fn verify_content_id_works() {
        let data = b"test content";
        let id = content_id_from_bytes(data);
        assert!(verify_content_id(data, &id));
        assert!(!verify_content_id(b"wrong content", &id));
    }

    #[test]
    fn empty_input() {
        let id = content_id_from_bytes(b"");
        // BLAKE3 of empty input is well-defined.
        assert_eq!(id.version(), 0x01);
        assert_eq!(id.hash_bytes().len(), 32);
    }
}
