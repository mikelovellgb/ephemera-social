//! Test infrastructure for the Ephemera decentralized social platform.
//!
//! This crate provides reusable test utilities for all Ephemera crates:
//!
//! - [`TestNode`] — a self-contained node with a temp directory and test config
//! - [`TestHarness`] — spin up N interconnected test nodes
//! - [`fixtures`] — random test data generators (posts, identities, media)
//! - [`assertions`] — domain-specific assertion helpers
//!
//! # Usage
//!
//! ```rust,no_run
//! use ephemera_test_utils::{TestNode, fixtures};
//!
//! #[tokio::test]
//! async fn example() {
//!     let node = TestNode::new().await.unwrap();
//!     let post = fixtures::random_text_post();
//!     let result = node.create_post(&post).await.unwrap();
//!     assert!(result.get("content_hash").is_some());
//! }
//! ```

pub mod assertions;
pub mod fixtures;
pub mod harness;
pub mod node;

pub use assertions::*;
pub use harness::TestHarness;
pub use node::TestNode;

/// Initialize tracing for tests (call once per test binary).
///
/// Sets up a subscriber that writes to stdout with `RUST_LOG` filtering.
/// Safe to call multiple times; subsequent calls are silently ignored.
pub fn init_test_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_test_writer()
        .try_init();
}
