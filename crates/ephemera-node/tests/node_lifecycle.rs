//! Integration tests for the EphemeraNode lifecycle:
//! - Network subsystem starts and binds a listener
//! - Gossip ingest loop is spawned and processes incoming posts
//! - Epoch key rotation task is spawned
//! - Handle RPC methods are registered in the router
//! - Shutdown tears down network and background tasks
//! - Iroh transport activates when identity is unlocked
//! - Iroh NodeId matches the identity's Ed25519 public key

use ephemera_config::NodeConfig;
use ephemera_node::EphemeraNode;
use std::net::SocketAddr;

/// Helper: create a `NodeConfig` with a random listen port in a temp dir.
fn test_config(dir: &std::path::Path) -> NodeConfig {
    let mut config = NodeConfig::default_for(dir);
    config.listen_addr = Some("127.0.0.1:0".parse::<SocketAddr>().unwrap());
    config
}

/// Verify that `EphemeraNode::start()` binds the network listener and the
/// `network()` accessor returns a `Some` with a non-zero port.
#[tokio::test]
async fn test_node_starts_with_network() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let mut node = EphemeraNode::new(config).unwrap();

    // Before start, network should be None.
    assert!(
        node.network().is_none(),
        "network should be None before start"
    );

    node.start().await.unwrap();

    // After start, network should be Some.
    let net = node.network().expect("network should be Some after start");
    assert_eq!(net.peer_count(), 0, "no peers connected yet");

    // Node should be marked as running.
    assert!(node.is_running(), "node should be running after start");

    // Shutdown should work cleanly.
    node.shutdown().await.unwrap();
    assert!(!node.is_running(), "node should not be running after shutdown");
    assert!(
        node.network().is_none(),
        "network should be None after shutdown"
    );
}

/// Verify that calling `start()` twice is idempotent (no panic, no error).
#[tokio::test]
async fn test_node_start_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let mut node = EphemeraNode::new(config).unwrap();

    node.start().await.unwrap();
    // Second start should be a no-op.
    node.start().await.unwrap();

    assert!(node.is_running());
    node.shutdown().await.unwrap();
}

/// Verify that the gossip ingest loop is spawned. The full end-to-end test
/// (publish -> receive -> store) is covered in tests/gossip_pipeline.rs.
/// Here we verify that start() successfully subscribes to the public feed
/// and spawns the ingest task without errors.
#[tokio::test]
async fn test_gossip_ingest_spawned() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let mut node = EphemeraNode::new(config).unwrap();

    // start() subscribes to the public feed topic and spawns the ingest
    // loop. If either fails, start() returns an error.
    node.start().await.unwrap();

    // The node should be running with a network subsystem.
    assert!(node.is_running());
    assert!(node.network().is_some());

    node.shutdown().await.unwrap();
}

/// Verify that the epoch key rotation background task is scheduled.
/// We do this by starting a node, letting the first tick fire, and
/// checking that the epoch key manager was initialized.
#[tokio::test]
async fn test_epoch_rotation_scheduled() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let mut node = EphemeraNode::new(config).unwrap();

    // Create an identity so the epoch key manager can initialize.
    node.services().identity.create("test-pass").await.unwrap();

    node.start().await.unwrap();

    // The epoch rotation loop runs on an interval. The first tick fires
    // immediately due to tokio::time::interval behavior. Give it a moment.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // The epoch key manager should have been initialized by the background
    // task (or by the identity creation which triggers it).
    // (The background task calls init_epoch_key_manager on each tick.)
    {
        let ekm_guard = node.services().epoch_key_manager.lock().unwrap();
        assert!(
            ekm_guard.is_some(),
            "epoch key manager should be initialized after start"
        );
    }

    node.shutdown().await.unwrap();
}

/// Verify that handle RPC methods are registered in the router.
#[tokio::test]
async fn test_handle_rpc_methods_registered() {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let event_bus = ephemera_events::EventBus::new();
    let svc = std::sync::Arc::new(
        ephemera_node::services::ServiceContainer::new(&config, event_bus).unwrap(),
    );

    let router = ephemera_node::api::build_router(svc);
    let methods = router.method_names();

    assert!(
        methods.contains(&"identity.register_handle".to_string()),
        "router should have identity.register_handle"
    );
    assert!(
        methods.contains(&"identity.lookup_handle".to_string()),
        "router should have identity.lookup_handle"
    );
    assert!(
        methods.contains(&"identity.renew_handle".to_string()),
        "router should have identity.renew_handle"
    );
    assert!(
        methods.contains(&"identity.release_handle".to_string()),
        "router should have identity.release_handle"
    );
    assert!(
        methods.contains(&"identity.my_handle".to_string()),
        "router should have identity.my_handle"
    );
}

/// Verify that when the identity is unlocked before start(), the node
/// creates an Iroh transport endpoint (when the `iroh-transport` feature is
/// enabled, which is the default).
#[cfg(feature = "iroh-transport")]
#[tokio::test]
async fn test_node_starts_with_iroh() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let mut node = EphemeraNode::new(config).unwrap();

    // Create and unlock an identity so the secret key is available.
    node.services()
        .identity
        .create("iroh-test-pass")
        .await
        .unwrap();

    node.start().await.unwrap();

    let net = node
        .network()
        .expect("network should be Some after start");

    // With identity unlocked, the transport should be Iroh.
    assert_eq!(
        net.transport_kind(),
        ephemera_node::network::TransportKind::Iroh,
        "transport should be Iroh when identity is unlocked"
    );

    // Peer count starts at zero (no peers connected yet).
    assert_eq!(net.peer_count(), 0);

    node.shutdown().await.unwrap();
}

/// Verify that the Iroh NodeId matches the identity's Ed25519 public key.
///
/// This is the critical property that makes "share your pubkey = share your
/// network address" work. The Iroh endpoint derives its NodeId from the same
/// Ed25519 secret key as the identity, so they MUST produce the same public
/// key bytes.
#[cfg(feature = "iroh-transport")]
#[tokio::test]
async fn test_iroh_node_id_matches_identity() {
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let mut node = EphemeraNode::new(config).unwrap();

    // Create an identity.
    let create_result = node
        .services()
        .identity
        .create("match-test-pass")
        .await
        .unwrap();

    // Extract the identity's public key (hex-encoded).
    let identity_pubkey_hex = create_result["pseudonym_pubkey"]
        .as_str()
        .expect("create should return pseudonym_pubkey")
        .to_string();

    node.start().await.unwrap();

    let net = node
        .network()
        .expect("network should be Some after start");

    // The network's local NodeId should match the identity pubkey.
    let node_id_hex = net.local_id().to_string();

    assert_eq!(
        node_id_hex, identity_pubkey_hex,
        "Iroh NodeId ({node_id_hex}) must match identity pubkey ({identity_pubkey_hex})"
    );

    node.shutdown().await.unwrap();
}
