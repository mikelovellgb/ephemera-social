//! Integration tests for the Ephemera platform.
//!
//! These tests exercise cross-crate interactions: creating a node, sending
//! JSON-RPC requests, and verifying end-to-end behavior.
//!
//! ## Test files
//!
//! | File | Package | Description |
//! |------|---------|-------------|
//! | `smoke_test.rs` | ephemera-test-utils | Node lifecycle, identity, posts, signatures |
//! | `multi_node.rs` | ephemera-node | Multi-node P2P integration (gossip, transport) |
//!
//! ## Running
//!
//! ```sh
//! # Smoke tests
//! cargo test -p ephemera-test-utils --test smoke_test
//!
//! # Multi-node integration tests
//! cargo test -p ephemera-node --test multi_node
//!
//! # All workspace tests
//! cargo test --workspace
//! ```
//!
//! ## Implemented test scenarios
//!
//! 1. Node lifecycle: create -> start -> status -> shutdown (smoke_test)
//! 2. Identity: create identity -> unlock -> get active pseudonym (smoke_test)
//! 3. Posts: create post -> get post -> verify content (smoke_test, core_loop)
//! 4. Feed: create posts -> fetch connections feed -> verify ordering (core_loop)
//! 5. Two-node post propagation (multi_node)
//! 6. Three-node gossip chain with forwarding (multi_node)
//! 7. Bidirectional post exchange (multi_node)
//! 8. Video/media post distribution (multi_node)
//! 9. Content dedup across network (multi_node)
//! 10. Node resilience: disconnect/reconnect (multi_node)
//! 11. Topic isolation (multi_node)
//! 12. Concurrent multi-publisher full mesh (multi_node)
//!
//! ## Planned scenarios (not yet implemented)
//!
//! - Social: send connection request -> accept -> list connections
//! - Messages: send DM -> list conversations -> get thread
//! - Moderation: report content -> block user -> verify filtering
//! - GC with real time-based expiry across multiple nodes
