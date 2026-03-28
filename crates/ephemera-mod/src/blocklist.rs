//! Hash-based content blocklist.
//!
//! Maintains a set of BLAKE3 content hashes that are known-bad. Content
//! whose hash appears in the blocklist is blocked from display and
//! propagation. The blocklist can be loaded from and saved to a simple
//! line-oriented hex file for persistence.

use std::collections::HashSet;

use crate::ModerationError;

/// A local blocklist of content hashes.
///
/// Stores BLAKE3 digests (32 bytes each) of content that should be
/// blocked. Lookups are O(1) via a `HashSet`.
pub struct LocalBlocklist {
    hashes: HashSet<[u8; 32]>,
}

impl LocalBlocklist {
    /// Create an empty blocklist.
    pub fn new() -> Self {
        Self {
            hashes: HashSet::new(),
        }
    }

    /// Add a content hash to the blocklist.
    ///
    /// Returns `true` if the hash was newly inserted, `false` if it was
    /// already present.
    pub fn add(&mut self, hash: [u8; 32]) -> bool {
        self.hashes.insert(hash)
    }

    /// Remove a content hash from the blocklist.
    ///
    /// Returns `true` if the hash was present and removed.
    pub fn remove(&mut self, hash: &[u8; 32]) -> bool {
        self.hashes.remove(hash)
    }

    /// Check whether a content hash is blocked.
    pub fn is_blocked(&self, hash: &[u8; 32]) -> bool {
        self.hashes.contains(hash)
    }

    /// Check content bytes: compute the BLAKE3 hash and check the blocklist.
    pub fn check_content(&self, content: &[u8]) -> bool {
        let hash = blake3::hash(content);
        self.is_blocked(hash.as_bytes())
    }

    /// Number of entries in the blocklist.
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    /// Whether the blocklist is empty.
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    /// Serialize the blocklist to a newline-delimited hex string.
    ///
    /// Each line is a 64-character hex-encoded BLAKE3 digest.
    pub fn save_to_string(&self) -> String {
        let mut lines: Vec<String> = self.hashes.iter().map(hex::encode).collect();
        lines.sort(); // Deterministic output
        lines.join("\n")
    }

    /// Load a blocklist from a newline-delimited hex string.
    ///
    /// Blank lines and lines that fail to parse are silently skipped.
    pub fn load_from_string(data: &str) -> Result<Self, ModerationError> {
        let mut hashes = HashSet::new();
        for line in data.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let bytes = hex::decode(trimmed).map_err(|e| ModerationError::BlocklistIo {
                reason: format!("invalid hex on line: {e}"),
            })?;
            if bytes.len() != 32 {
                return Err(ModerationError::BlocklistIo {
                    reason: format!("expected 32 bytes, got {}", bytes.len()),
                });
            }
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&bytes);
            hashes.insert(hash);
        }
        Ok(Self { hashes })
    }
}

impl Default for LocalBlocklist {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_a() -> [u8; 32] {
        *blake3::hash(b"bad content A").as_bytes()
    }

    fn hash_b() -> [u8; 32] {
        *blake3::hash(b"bad content B").as_bytes()
    }

    #[test]
    fn add_and_check() {
        let mut bl = LocalBlocklist::new();
        assert!(!bl.is_blocked(&hash_a()));

        bl.add(hash_a());
        assert!(bl.is_blocked(&hash_a()));
        assert!(!bl.is_blocked(&hash_b()));
    }

    #[test]
    fn add_returns_insertion_status() {
        let mut bl = LocalBlocklist::new();
        assert!(bl.add(hash_a())); // new
        assert!(!bl.add(hash_a())); // duplicate
    }

    #[test]
    fn remove_entry() {
        let mut bl = LocalBlocklist::new();
        bl.add(hash_a());
        assert!(bl.remove(&hash_a()));
        assert!(!bl.is_blocked(&hash_a()));
        assert!(!bl.remove(&hash_a())); // already removed
    }

    #[test]
    fn check_content_by_bytes() {
        let mut bl = LocalBlocklist::new();
        let content = b"this is bad content";
        let hash = *blake3::hash(content).as_bytes();
        bl.add(hash);

        assert!(bl.check_content(content));
        assert!(!bl.check_content(b"this is fine"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut bl = LocalBlocklist::new();
        bl.add(hash_a());
        bl.add(hash_b());

        let serialized = bl.save_to_string();
        let loaded = LocalBlocklist::load_from_string(&serialized).unwrap();

        assert_eq!(loaded.len(), 2);
        assert!(loaded.is_blocked(&hash_a()));
        assert!(loaded.is_blocked(&hash_b()));
    }

    #[test]
    fn load_skips_blank_lines() {
        let data = format!("{}\n\n{}\n", hex::encode(hash_a()), hex::encode(hash_b()));
        let bl = LocalBlocklist::load_from_string(&data).unwrap();
        assert_eq!(bl.len(), 2);
    }

    #[test]
    fn load_rejects_bad_hex() {
        let result = LocalBlocklist::load_from_string("not_valid_hex!");
        assert!(result.is_err());
    }

    #[test]
    fn load_rejects_wrong_length() {
        let result = LocalBlocklist::load_from_string("aabbccdd");
        assert!(result.is_err());
    }

    #[test]
    fn empty_blocklist() {
        let bl = LocalBlocklist::new();
        assert!(bl.is_empty());
        assert_eq!(bl.len(), 0);
        assert_eq!(bl.save_to_string(), "");
    }
}
