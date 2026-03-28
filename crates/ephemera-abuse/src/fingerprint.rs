//! SimHash-based near-duplicate content detection.
//!
//! Uses SimHash to generate 64-bit fingerprints of text content.
//! Near-duplicate posts (Hamming distance <= 3) are flagged as potential
//! spam. This catches copy-paste spam even with minor modifications.

use std::collections::VecDeque;

/// Maximum number of recent fingerprints to retain.
const MAX_RECENT_FINGERPRINTS: usize = 10_000;

/// Hamming distance threshold for considering content near-duplicate.
const NEAR_DUPLICATE_THRESHOLD: u32 = 3;

/// A 64-bit SimHash fingerprint of text content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentFingerprint(u64);

impl ContentFingerprint {
    /// Compute the SimHash fingerprint of the given text.
    ///
    /// The algorithm:
    /// 1. Tokenize text by whitespace.
    /// 2. Hash each token with BLAKE3 (truncated to 64 bits).
    /// 3. For each bit position, sum +1 if set, -1 otherwise.
    /// 4. The fingerprint bit is 1 if the sum is positive, 0 otherwise.
    pub fn compute(text: &str) -> Self {
        let mut v = [0i32; 64];
        for token in text.split_whitespace() {
            let hash = Self::hash_token(token);
            for (i, slot) in v.iter_mut().enumerate() {
                if hash & (1u64 << i) != 0 {
                    *slot += 1;
                } else {
                    *slot -= 1;
                }
            }
        }
        let mut fingerprint: u64 = 0;
        for (i, &count) in v.iter().enumerate() {
            if count > 0 {
                fingerprint |= 1u64 << i;
            }
        }
        Self(fingerprint)
    }

    /// Return the raw 64-bit fingerprint value.
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Compute the Hamming distance to another fingerprint.
    pub fn hamming_distance(self, other: Self) -> u32 {
        (self.0 ^ other.0).count_ones()
    }

    /// Whether this fingerprint is a near-duplicate of another.
    pub fn is_near_duplicate(self, other: Self) -> bool {
        self.hamming_distance(other) <= NEAR_DUPLICATE_THRESHOLD
    }

    /// Hash a single token to 64 bits using BLAKE3.
    fn hash_token(token: &str) -> u64 {
        let hash = blake3::hash(token.as_bytes());
        let bytes = hash.as_bytes();
        u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }
}

/// A rolling window of recent content fingerprints for spam detection.
///
/// Stores the most recent fingerprints and checks new content against
/// them for near-duplicates.
pub struct FingerprintStore {
    recent: VecDeque<ContentFingerprint>,
    max_size: usize,
}

impl FingerprintStore {
    /// Create a new fingerprint store with the default capacity.
    pub fn new() -> Self {
        Self {
            recent: VecDeque::with_capacity(MAX_RECENT_FINGERPRINTS),
            max_size: MAX_RECENT_FINGERPRINTS,
        }
    }

    /// Create a fingerprint store with a custom capacity.
    pub fn with_capacity(max_size: usize) -> Self {
        Self {
            recent: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    /// Check if the given text is a near-duplicate of any recent content.
    ///
    /// Returns `Some(fingerprint)` of the matching entry if found.
    pub fn check_duplicate(&self, text: &str) -> Option<ContentFingerprint> {
        let fp = ContentFingerprint::compute(text);
        self.recent
            .iter()
            .find(|&&existing| fp.is_near_duplicate(existing))
            .copied()
    }

    /// Record a content fingerprint. Evicts oldest if full.
    pub fn record(&mut self, text: &str) -> ContentFingerprint {
        let fp = ContentFingerprint::compute(text);
        if self.recent.len() >= self.max_size {
            self.recent.pop_front();
        }
        self.recent.push_back(fp);
        fp
    }

    /// Check for duplicates and record in one step.
    ///
    /// Returns `true` if the content is a near-duplicate.
    pub fn check_and_record(&mut self, text: &str) -> bool {
        let is_dup = self.check_duplicate(text).is_some();
        self.record(text);
        is_dup
    }

    /// Number of fingerprints currently stored.
    pub fn len(&self) -> usize {
        self.recent.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.recent.is_empty()
    }

    /// Clear all stored fingerprints.
    pub fn clear(&mut self) {
        self.recent.clear();
    }
}

impl Default for FingerprintStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_same_fingerprint() {
        let a = ContentFingerprint::compute("hello world foo bar");
        let b = ContentFingerprint::compute("hello world foo bar");
        assert_eq!(a, b);
    }

    #[test]
    fn different_text_different_fingerprint() {
        let a = ContentFingerprint::compute("the quick brown fox jumps");
        let b = ContentFingerprint::compute("completely unrelated content here today");
        assert!(a.hamming_distance(b) > NEAR_DUPLICATE_THRESHOLD);
    }

    #[test]
    fn hamming_distance_properties() {
        let a = ContentFingerprint(0);
        let b = ContentFingerprint(u64::MAX);
        assert_eq!(a.hamming_distance(b), 64);
        assert_eq!(a.hamming_distance(a), 0);
    }

    #[test]
    fn store_detects_duplicate() {
        let mut store = FingerprintStore::new();
        let text = "this is a test message for duplicate detection";
        assert!(!store.check_and_record(text));
        assert!(store.check_and_record(text));
    }

    #[test]
    fn store_allows_unique_content() {
        let mut store = FingerprintStore::new();
        assert!(!store.check_and_record("first unique message about topic alpha"));
        assert!(!store.check_and_record("second completely different message beta"));
    }

    #[test]
    fn store_evicts_oldest() {
        let mut store = FingerprintStore::with_capacity(3);
        store.record("first message alpha");
        store.record("second message beta");
        store.record("third message gamma");
        assert_eq!(store.len(), 3);
        store.record("fourth message delta");
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn empty_text_produces_zero_fingerprint() {
        let fp = ContentFingerprint::compute("");
        assert_eq!(fp.as_u64(), 0);
    }
}
