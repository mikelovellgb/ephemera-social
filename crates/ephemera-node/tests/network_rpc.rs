//! Integration tests for the network.* RPC methods and bootstrap behavior.
//!
//! NOTE: These tests were written for the old TCP transport and need rework
//! for Iroh-only networking. They are marked `#[ignore]` until the test
//! harness supports Iroh relay-based connections.
//!
//! Tests:
//! - `test_connect_to_remote_peer_via_rpc` -- call network.connect, verify peer added
//! - `test_bootstrap_connects_on_startup` -- configure bootstrap nodes, verify connected
//! - `test_peers_rpc_lists_connected` -- connect a peer, call network.peers, verify listed

use ephemera_config::NodeConfig;
use ephemera_node::api::build_router_with_network;
use ephemera_node::network::NetworkSubsystem;
use ephemera_node::rpc::JsonRpcRequest;
use ephemera_node::EphemeraNode;
use ephemera_types::NodeId;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;

/// Helper: create a `NodeConfig` with a random listen port in a temp dir.
fn test_config(dir: &std::path::Path) -> NodeConfig {
    let mut config = NodeConfig::default_for(dir);
    config.listen_addr = Some("127.0.0.1:0".parse::<SocketAddr>().unwrap());
    config
}

/// Helper: create a JSON-RPC request.
fn make_request(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: Value::Number(1.into()),
    }
}

/// Call `network.connect` RPC to connect to a remote peer, verify the peer
/// is added to the network.
///
/// TODO: Rework for Iroh-only transport. Old TCP transport has been removed.
#[ignore]
#[tokio::test]
async fn test_connect_to_remote_peer_via_rpc() {
    // Start a standalone network subsystem as the "remote" peer.
    let remote_net = NetworkSubsystem::new_random().await.unwrap();

    // Start a full EphemeraNode as the "local" node.
    let dir = tempfile::tempdir().unwrap();
    let config = test_config(dir.path());
    let mut node = EphemeraNode::new(config).unwrap();
    node.start().await.unwrap();

    // Build the router with network support. The network is set on the
    // ServiceContainer so dynamic handlers read from services.network (Mutex).
    let services = Arc::new({
        let cfg = node.config().clone();
        let event_bus = node.event_bus().clone();
        ephemera_node::services::ServiceContainer::new(&cfg, event_bus).unwrap()
    });
    if let Some(net) = node.network().cloned() {
        services.set_network(net);
    }
    let router = build_router_with_network(services, None);

    // Call network.connect RPC.
    // TODO: Need Iroh node address instead of TCP addr
    let req = make_request(
        "network.connect",
        serde_json::json!({ "addr": "placeholder" }),
    );
    let resp = router.dispatch(req).await;
    assert!(
        resp.error.is_none(),
        "network.connect should succeed, got: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    assert_eq!(result["ok"], true, "should return ok: true");
    assert!(
        result.get("peer_id").is_some(),
        "should return a peer_id"
    );

    // Verify the peer is actually connected.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let net = node.network().unwrap();
    assert!(
        net.peer_count() >= 1,
        "node should have at least 1 connected peer after network.connect"
    );

    node.shutdown().await.unwrap();
    remote_net.shutdown().await;
}

/// Configure bootstrap nodes in the config, start the node, verify it
/// connected to the bootstrap peers automatically.
///
/// TODO: Rework for Iroh-only transport. Old TCP transport has been removed.
#[ignore]
#[tokio::test]
async fn test_bootstrap_connects_on_startup() {
    // Start a standalone network subsystem as the "bootstrap" peer.
    let bootstrap_net = NetworkSubsystem::new_random().await.unwrap();

    // Create a node config with the bootstrap address.
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    // TODO: Need Iroh node address instead of TCP addr
    config.bootstrap_nodes = vec![];

    let mut node = EphemeraNode::new(config).unwrap();
    node.start().await.unwrap();

    // Give bootstrap connection time to complete.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // The node should have connected to the bootstrap peer.
    let net = node.network().unwrap();
    assert!(
        net.peer_count() >= 1,
        "node should have at least 1 peer after bootstrapping (got {})",
        net.peer_count()
    );

    // The bootstrap node should also see the incoming connection.
    assert!(
        bootstrap_net.peer_count() >= 1,
        "bootstrap node should see the incoming connection (got {})",
        bootstrap_net.peer_count()
    );

    node.shutdown().await.unwrap();
    bootstrap_net.shutdown().await;
}

/// Connect a peer, then call `network.peers` RPC, verify the peer appears
/// in the list.
///
/// TODO: Rework for Iroh-only transport. Old TCP transport has been removed.
#[ignore]
#[tokio::test]
async fn test_peers_rpc_lists_connected() {
    // Start a standalone network subsystem as the "remote" peer.
    let remote_net = NetworkSubsystem::new_random().await.unwrap();

    // Start a full EphemeraNode.
    let dir = tempfile::tempdir().unwrap();
    let mut config = test_config(dir.path());
    // TODO: Need Iroh node address instead of TCP addr
    config.bootstrap_nodes = vec![];

    let mut node = EphemeraNode::new(config).unwrap();
    node.start().await.unwrap();

    // Wait for bootstrap connection.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Build the router with network support. The network is set on the
    // ServiceContainer so dynamic handlers read from services.network (Mutex).
    let services = Arc::new({
        let cfg = node.config().clone();
        let event_bus = node.event_bus().clone();
        ephemera_node::services::ServiceContainer::new(&cfg, event_bus).unwrap()
    });
    if let Some(net) = node.network().cloned() {
        services.set_network(net);
    }
    let router = build_router_with_network(services, None);

    // Call network.peers RPC.
    let req = make_request("network.peers", serde_json::json!({}));
    let resp = router.dispatch(req).await;
    assert!(
        resp.error.is_none(),
        "network.peers should succeed, got: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    let count = result["count"].as_u64().unwrap_or(0);
    assert!(
        count >= 1,
        "network.peers should list at least 1 peer (got {count})"
    );

    let peers = result["peers"].as_array().unwrap();
    assert!(
        !peers.is_empty(),
        "peers array should not be empty"
    );

    // Verify the peer_id field is present and non-empty.
    let first_peer = &peers[0];
    let peer_id = first_peer["peer_id"].as_str().unwrap_or("");
    assert!(
        !peer_id.is_empty(),
        "peer_id should be a non-empty hex string"
    );

    node.shutdown().await.unwrap();
    remote_net.shutdown().await;
}
