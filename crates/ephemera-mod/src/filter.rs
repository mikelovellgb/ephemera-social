//! Content filtering pipeline.
//!
//! Before content is published or displayed, it passes through a series
//! of moderation checks. The `ContentFilter` aggregates these checks
//! and returns a verdict: allow, block, or require human review.

use ephemera_types::ContentId;
use serde::{Deserialize, Serialize};

use crate::blocklist::LocalBlocklist;

/// The result of running content through the moderation filter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterResult {
    /// Content passes all checks; safe to publish or display.
    Allow,
    /// Content is blocked with a reason.
    Block(String),
    /// Content needs manual review (post-MVP: routed to moderation quorum).
    RequireReview(String),
}

/// Content filter that aggregates moderation checks.
///
/// Currently checks:
/// - Content hash against the local blocklist
/// - Basic text heuristics (excessive caps, known spam patterns)
///
/// Phase 2 will add:
/// - CSAM perceptual hash bloom filter (via `ephemera-media`)
/// - Community moderation quorum verdicts
pub struct ContentFilter {
    blocklist: LocalBlocklist,
    /// Maximum allowed ratio of uppercase characters (0.0 to 1.0).
    max_caps_ratio: f64,
    /// Minimum text length to trigger caps ratio check.
    min_length_for_caps_check: usize,
}

impl ContentFilter {
    /// Create a content filter with a given blocklist.
    pub fn new(blocklist: LocalBlocklist) -> Self {
        Self {
            blocklist,
            max_caps_ratio: 0.8,
            min_length_for_caps_check: 20,
        }
    }

    /// Create a content filter with an empty blocklist.
    pub fn empty() -> Self {
        Self::new(LocalBlocklist::new())
    }

    /// Replace the blocklist.
    pub fn set_blocklist(&mut self, blocklist: LocalBlocklist) {
        self.blocklist = blocklist;
    }

    /// Return a reference to the current blocklist.
    pub fn blocklist(&self) -> &LocalBlocklist {
        &self.blocklist
    }

    /// Return a mutable reference to the current blocklist.
    pub fn blocklist_mut(&mut self) -> &mut LocalBlocklist {
        &mut self.blocklist
    }

    /// Run content through all moderation checks.
    ///
    /// `content_hash` is the BLAKE3 hash of the content (for blocklist
    /// checking). `text` is the plaintext body (for heuristic checks).
    pub fn check(&self, content_hash: &ContentId, text: &str) -> FilterResult {
        // Check 1: blocklist hash match
        if self
            .blocklist
            .is_blocked(&content_hash_to_digest(content_hash))
        {
            return FilterResult::Block("content hash is blocklisted".into());
        }

        // Check 2: excessive uppercase (potential shouting/spam)
        if let Some(result) = self.check_excessive_caps(text) {
            return result;
        }

        // Check 3: repeated character spam
        if let Some(result) = self.check_repeated_chars(text) {
            return result;
        }

        FilterResult::Allow
    }

    /// Check text-only content (no pre-computed hash).
    ///
    /// Computes the content hash internally and runs all checks.
    pub fn check_text(&self, text: &str) -> FilterResult {
        let digest = *blake3::hash(text.as_bytes()).as_bytes();
        let hash = ContentId::from_digest(digest);
        self.check(&hash, text)
    }

    /// Detect excessive uppercase text (shouting).
    fn check_excessive_caps(&self, text: &str) -> Option<FilterResult> {
        if text.len() < self.min_length_for_caps_check {
            return None;
        }

        let alpha_chars: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
        if alpha_chars.is_empty() {
            return None;
        }

        let upper_count = alpha_chars.iter().filter(|c| c.is_uppercase()).count();
        let ratio = upper_count as f64 / alpha_chars.len() as f64;

        if ratio > self.max_caps_ratio {
            Some(FilterResult::RequireReview(
                "excessive uppercase detected".into(),
            ))
        } else {
            None
        }
    }

    /// Detect repeated character spam (e.g., "aaaaaaaaaa").
    fn check_repeated_chars(&self, text: &str) -> Option<FilterResult> {
        let chars: Vec<char> = text.chars().collect();
        if chars.len() < 20 {
            return None;
        }

        // Count the longest run of the same character
        let mut max_run = 1u32;
        let mut current_run = 1u32;
        for window in chars.windows(2) {
            if window[0] == window[1] {
                current_run += 1;
                if current_run > max_run {
                    max_run = current_run;
                }
            } else {
                current_run = 1;
            }
        }

        // If more than half the text is a single repeated character, flag it
        if max_run as usize > chars.len() / 2 {
            Some(FilterResult::Block(
                "repeated character spam detected".into(),
            ))
        } else {
            None
        }
    }
}

/// Extract the 32-byte digest from a `ContentId`.
fn content_hash_to_digest(hash: &ContentId) -> [u8; 32] {
    *hash.hash_bytes()
}

impl Default for ContentFilter {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hash(content: &[u8]) -> ContentId {
        ContentId::from_digest(*blake3::hash(content).as_bytes())
    }

    #[test]
    fn clean_content_allowed() {
        let filter = ContentFilter::empty();
        let result = filter.check_text("Hello, this is a normal post about my day.");
        assert_eq!(result, FilterResult::Allow);
    }

    #[test]
    fn blocklisted_content_blocked() {
        let mut filter = ContentFilter::empty();
        let content = b"this content is banned";
        let hash = *blake3::hash(content).as_bytes();
        filter.blocklist_mut().add(hash);

        let content_hash = make_hash(content);
        let result = filter.check(&content_hash, "this content is banned");
        assert!(matches!(result, FilterResult::Block(_)));
    }

    #[test]
    fn excessive_caps_flagged() {
        let filter = ContentFilter::empty();
        let result = filter.check_text("THIS IS ALL CAPS SHOUTING REALLY LOUDLY AT EVERYONE");
        assert!(matches!(result, FilterResult::RequireReview(_)));
    }

    #[test]
    fn normal_caps_allowed() {
        let filter = ContentFilter::empty();
        let result = filter.check_text("This is Normal Text With Some Caps");
        assert_eq!(result, FilterResult::Allow);
    }

    #[test]
    fn short_text_skips_caps_check() {
        let filter = ContentFilter::empty();
        let result = filter.check_text("OK SURE");
        assert_eq!(result, FilterResult::Allow);
    }

    #[test]
    fn repeated_chars_blocked() {
        let filter = ContentFilter::empty();
        let result = filter.check_text("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(matches!(result, FilterResult::Block(_)));
    }

    #[test]
    fn varied_text_not_flagged_as_repeated() {
        let filter = ContentFilter::empty();
        let result = filter.check_text("the quick brown fox jumps over the lazy dog today");
        assert_eq!(result, FilterResult::Allow);
    }

    #[test]
    fn empty_text_allowed() {
        let filter = ContentFilter::empty();
        let result = filter.check_text("");
        assert_eq!(result, FilterResult::Allow);
    }

    #[test]
    fn filter_with_custom_blocklist() {
        let mut blocklist = LocalBlocklist::new();
        let bad_hash = *blake3::hash(b"spam").as_bytes();
        blocklist.add(bad_hash);

        let filter = ContentFilter::new(blocklist);
        assert_eq!(filter.blocklist().len(), 1);
    }

    #[test]
    fn set_blocklist_replaces() {
        let mut filter = ContentFilter::empty();
        assert!(filter.blocklist().is_empty());

        let mut new_bl = LocalBlocklist::new();
        new_bl.add([0xAA; 32]);
        filter.set_blocklist(new_bl);
        assert_eq!(filter.blocklist().len(), 1);
    }
}
