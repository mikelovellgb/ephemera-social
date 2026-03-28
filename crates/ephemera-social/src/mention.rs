//! Mention extraction and resolution.
//!
//! Extracts `@handle` patterns from post bodies, resolves them to pubkeys
//! via the handle registry, and provides a query interface for "posts where
//! I am mentioned".

use serde::{Deserialize, Serialize};

/// A resolved mention within a post body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mention {
    /// Hex-encoded pubkey of the mentioned user.
    pub mentioned_pubkey: String,
    /// The display hint text (handle or name at time of mention).
    pub display_hint: String,
    /// Start byte offset in the post body.
    pub byte_start: usize,
    /// End byte offset in the post body.
    pub byte_end: usize,
}

/// Extract `@handle` patterns from a post body.
///
/// Returns a list of `(handle_name, byte_start, byte_end)` tuples.
/// The handle name does NOT include the `@` prefix.
#[must_use]
pub fn extract_mention_patterns(body: &str) -> Vec<(String, usize, usize)> {
    let mut mentions = Vec::new();
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'@' {
            let start = i;
            i += 1; // skip @
            // Handle characters: alphanumeric, dash, underscore
            let handle_start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
            {
                i += 1;
            }
            let handle_end = i;
            if handle_end > handle_start {
                let handle = body[handle_start..handle_end].to_string();
                mentions.push((handle, start, handle_end));
            }
        } else {
            i += 1;
        }
    }

    // Limit to 20 mentions per post (per spec).
    mentions.truncate(20);
    mentions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_single_mention() {
        let mentions = extract_mention_patterns("hello @alice how are you");
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].0, "alice");
        assert_eq!(mentions[0].1, 6); // byte offset of @
        assert_eq!(mentions[0].2, 12); // byte offset after "alice"
    }

    #[test]
    fn extract_multiple_mentions() {
        let mentions = extract_mention_patterns("@alice and @bob-42 and @carol_x");
        assert_eq!(mentions.len(), 3);
        assert_eq!(mentions[0].0, "alice");
        assert_eq!(mentions[1].0, "bob-42");
        assert_eq!(mentions[2].0, "carol_x");
    }

    #[test]
    fn no_mentions() {
        let mentions = extract_mention_patterns("hello world, no mentions here");
        assert!(mentions.is_empty());
    }

    #[test]
    fn at_sign_alone_not_a_mention() {
        let mentions = extract_mention_patterns("email: foo@ bar");
        // "@" followed by space should not be a mention
        assert!(mentions.is_empty());
    }

    #[test]
    fn limit_20_mentions() {
        let body = (0..25).map(|i| format!("@user{i}")).collect::<Vec<_>>().join(" ");
        let mentions = extract_mention_patterns(&body);
        assert_eq!(mentions.len(), 20);
    }
}
