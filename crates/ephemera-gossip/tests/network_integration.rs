//! Network integration tests for gossip.
//!
//! These tests previously used TCP transport which has been removed.
//! Gossip local publish/subscribe is tested in service_tests.rs.
//! Real P2P gossip propagation is tested through the Iroh transport
//! integration tests in the ephemera-transport crate.
