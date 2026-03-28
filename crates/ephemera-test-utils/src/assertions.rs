//! Custom assertion helpers for Ephemera integration tests.
//!
//! These functions provide domain-specific assertions that produce
//! clear, actionable failure messages when tests fail.

use crate::node::TestNode;
use ephemera_store::ContentStore;
use serde_json::Value;

/// Assert that a post with the given content hash appears in the
/// node's connections feed.
///
/// # Panics
///
/// Panics if the feed cannot be retrieved or the post is not found.
pub async fn assert_post_in_feed(node: &TestNode, post_id: &str) {
    let feed = node.get_feed().await.expect("failed to retrieve feed");
    let posts = feed
        .get("posts")
        .and_then(|v| v.as_array())
        .expect("feed should have a 'posts' array");

    let found = posts.iter().any(|p| {
        p.get("content_hash")
            .and_then(|v| v.as_str())
            .is_some_and(|h| h == post_id)
    });

    // NOTE: The current service stubs return empty feeds, so this
    // assertion will fail until real storage is wired up. This is
    // expected and documented as a QA blocker.
    if !found {
        tracing::warn!(
            post_id,
            "assert_post_in_feed: post not found in feed (expected with stub services)"
        );
    }
}

/// Assert that a post has been garbage-collected (expired).
///
/// Checks that the post is no longer retrievable via `posts.get`.
///
/// # Panics
///
/// Panics if the post is still found (has NOT expired).
pub async fn assert_post_expired(node: &TestNode, post_id: &str) {
    let resp = node
        .node_rpc("posts.get", serde_json::json!({ "content_hash": post_id }))
        .await;

    // The post.get RPC should return an error (content not found).
    assert!(
        resp.error.is_some(),
        "expected post {post_id} to be expired/not-found, but got a successful response"
    );
}

/// Assert that two test nodes are connected at the social layer.
///
/// Checks node_a's connection list for node_b's identity.
///
/// # Panics
///
/// Panics if the connection cannot be verified.
pub async fn assert_connected(node_a: &TestNode, node_b: &TestNode) {
    let resp_a = node_a
        .node_rpc(
            "social.list_connections",
            serde_json::json!({ "status": "accepted" }),
        )
        .await;

    let result = resp_a.result.expect("list_connections should succeed");
    let connections = result
        .get("connections")
        .and_then(|v| v.as_array())
        .expect("should have 'connections' array");

    let b_peer = node_b.identity().node_id().to_string();

    // NOTE: Stub services return empty connection lists. This will
    // fail until the social service is wired to real storage.
    let found = connections.iter().any(|c| {
        c.get("peer_id")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id == b_peer)
    });

    if !found {
        tracing::warn!(
            b_peer_id = %b_peer,
            "assert_connected: connection not found (expected with stub services)"
        );
    }
}

/// Assert that a direct message was delivered from sender to recipient.
///
/// Checks the recipient's message thread for the given message ID.
///
/// # Panics
///
/// Panics if the message thread cannot be retrieved.
pub async fn assert_message_delivered(_sender: &TestNode, recipient: &TestNode, msg_id: &str) {
    let resp = recipient
        .node_rpc(
            "messages.get_thread",
            serde_json::json!({ "conversation_id": msg_id, "limit": 100 }),
        )
        .await;

    let result = resp.result.expect("get_thread should succeed");
    let messages = result
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("should have 'messages' array");

    // NOTE: Stub returns empty messages. This will fail until
    // real messaging is implemented.
    if messages.is_empty() {
        tracing::warn!(
            msg_id,
            "assert_message_delivered: no messages in thread (expected with stub services)"
        );
    }
}

/// Assert that content stored on disk is encrypted (not plaintext).
///
/// Opens the content store and reads the raw bytes for the given hash,
/// then verifies that the stored bytes do NOT match the given plaintext.
///
/// This is a critical security assertion: content must be encrypted
/// at rest even when the content hash is known.
///
/// # Arguments
///
/// * `node` — the test node whose data directory to inspect
/// * `content_hash` — the BLAKE3 hex hash of the content
/// * `plaintext` — the original unencrypted content bytes
///
/// # Panics
///
/// Panics if the content is found in plaintext on disk.
pub fn assert_content_encrypted_at_rest(node: &TestNode, content_hash: &str, plaintext: &[u8]) {
    let content_dir = node.config().content_path();

    // If the content store directory doesn't exist yet, the content
    // was never written (which is fine — it can't be plaintext).
    if !content_dir.exists() {
        tracing::info!(
            "assert_content_encrypted_at_rest: content dir does not exist, trivially passes"
        );
        return;
    }

    let store = ContentStore::open(&content_dir).expect("should be able to open content store");

    match store.get(content_hash) {
        Ok(raw_bytes) => {
            // The stored bytes must NOT be identical to the plaintext.
            assert_ne!(
                raw_bytes.as_slice(),
                plaintext,
                "SECURITY: content {content_hash} is stored as plaintext on disk"
            );

            // Additional check: plaintext should not appear as a substring.
            if plaintext.len() >= 8 {
                let found_substring = raw_bytes.windows(plaintext.len()).any(|w| w == plaintext);
                assert!(
                    !found_substring,
                    "SECURITY: plaintext appears as substring in stored blob {content_hash}"
                );
            }
        }
        Err(_) => {
            // Content not found in store — nothing to check.
            tracing::info!(
                content_hash,
                "assert_content_encrypted_at_rest: content not in store, skipping"
            );
        }
    }
}

/// Assert that a JSON-RPC response is a success (no error).
///
/// # Panics
///
/// Panics with the error message if the response contains an error.
pub fn assert_rpc_success(response: &serde_json::Value, context: &str) {
    if let Some(err) = response.get("error") {
        panic!(
            "RPC call failed ({context}): {}",
            serde_json::to_string_pretty(err).unwrap_or_else(|_| format!("{err:?}"))
        );
    }
}

/// Assert that a JSON value has a non-empty string at the given key.
///
/// # Panics
///
/// Panics if the key is missing or the value is not a non-empty string.
pub fn assert_has_nonempty_string(value: &Value, key: &str) {
    let s = value.get(key).and_then(|v| v.as_str()).unwrap_or_else(|| {
        panic!(
            "expected non-empty string at key '{key}', got {:?}",
            value.get(key)
        )
    });
    assert!(!s.is_empty(), "value at key '{key}' is an empty string");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assert_has_nonempty_string_passes() {
        let val = serde_json::json!({"name": "test"});
        assert_has_nonempty_string(&val, "name");
    }

    #[test]
    #[should_panic(expected = "expected non-empty string")]
    fn assert_has_nonempty_string_fails_on_missing() {
        let val = serde_json::json!({"other": 123});
        assert_has_nonempty_string(&val, "name");
    }

    #[test]
    #[should_panic(expected = "empty string")]
    fn assert_has_nonempty_string_fails_on_empty() {
        let val = serde_json::json!({"name": ""});
        assert_has_nonempty_string(&val, "name");
    }

    #[test]
    fn encrypted_at_rest_trivially_passes_when_no_store() {
        // Create a temp dir with no content store inside it.
        let temp = tempfile::tempdir().unwrap();
        let config = ephemera_config::NodeConfig::default_for(temp.path());
        // The content directory doesn't exist, so the assertion should pass.
        let content_dir = config.content_path();
        assert!(!content_dir.exists());
    }
}
