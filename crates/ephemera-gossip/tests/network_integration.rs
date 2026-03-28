//! Integration tests proving two (or more) nodes can talk to each other
//! over real TCP connections using the gossip layer.

use ephemera_gossip::service::EagerGossipService;
use ephemera_gossip::topic::GossipTopic;
use ephemera_transport::tcp::TcpTransport;
use ephemera_transport::{PeerAddr, Transport};
use ephemera_types::NodeId;
use std::sync::Arc;
use std::time::Duration;

/// Helper: create a transport, listen on a random port, return (transport, addr).
async fn make_node(id_byte: u8) -> (Arc<TcpTransport>, NodeId, std::net::SocketAddr) {
    let id = NodeId::from_bytes([id_byte; 32]);
    let transport = Arc::new(TcpTransport::new(id));
    let addr = transport.listen("127.0.0.1:0").await.unwrap();
    (transport, id, addr)
}

/// Test 1: Start two nodes on different ports, connect them, verify they see each other.
#[tokio::test]
async fn test_two_nodes_connect() {
    let (transport_a, id_a, addr_a) = make_node(1).await;
    let (transport_b, id_b, _addr_b) = make_node(2).await;

    // B connects to A.
    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    // Give the listener time to accept and complete handshake.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // B should know about A.
    assert!(
        transport_b.is_connected(&id_a),
        "B should be connected to A"
    );

    // A should know about B (accepted the incoming connection).
    assert!(
        transport_a.is_connected(&id_b),
        "A should be connected to B"
    );

    // Verify peer counts.
    assert_eq!(transport_a.connected_peers().len(), 1);
    assert_eq!(transport_b.connected_peers().len(), 1);

    transport_a.shutdown();
    transport_b.shutdown();
}

/// Test 2: Node A creates a post (publishes via gossip), verify it appears on Node B.
#[tokio::test]
async fn test_post_propagation() {
    let (transport_a, id_a, addr_a) = make_node(10).await;
    let (transport_b, _id_b, _addr_b) = make_node(20).await;

    // B connects to A.
    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Create gossip services.
    let gossip_a = EagerGossipService::new(id_a, Arc::clone(&transport_a));
    let gossip_b = EagerGossipService::new(_id_b, Arc::clone(&transport_b));

    // Both subscribe to the public feed topic.
    let topic = GossipTopic::public_feed();
    let _sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let mut sub_b = gossip_b.subscribe(&topic).await.unwrap();

    // A publishes a "post".
    let post_content = b"Hello from Node A! This is my first post.".to_vec();
    gossip_a
        .publish(&topic, post_content.clone())
        .await
        .unwrap();

    // B should receive it.
    let received = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout waiting for post on Node B")
        .expect("subscription channel closed");

    assert_eq!(
        received.payload, post_content,
        "Node B should receive exactly the post content from Node A"
    );
    assert_eq!(
        received.source_node,
        *id_a.as_bytes(),
        "Origin should be Node A"
    );

    transport_a.shutdown();
    transport_b.shutdown();
}

/// Test 3: Bidirectional gossip -- A posts, B posts, both see each other's posts.
#[tokio::test]
async fn test_bidirectional() {
    let (transport_a, id_a, addr_a) = make_node(30).await;
    let (transport_b, id_b, _addr_b) = make_node(40).await;

    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let gossip_a = EagerGossipService::new(id_a, Arc::clone(&transport_a));
    let gossip_b = EagerGossipService::new(id_b, Arc::clone(&transport_b));

    let topic = GossipTopic::public_feed();
    let mut sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let mut sub_b = gossip_b.subscribe(&topic).await.unwrap();

    // A publishes.
    gossip_a
        .publish(&topic, b"Post from A".to_vec())
        .await
        .unwrap();

    // B receives A's post.
    let msg_on_b = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout: B waiting for A's post")
        .expect("B sub closed");
    assert_eq!(msg_on_b.payload, b"Post from A");

    // A also gets its own post locally (we deliver locally on publish).
    let msg_on_a_local = tokio::time::timeout(Duration::from_secs(1), sub_a.recv())
        .await
        .expect("timeout: A waiting for local echo")
        .expect("A sub closed");
    assert_eq!(msg_on_a_local.payload, b"Post from A");

    // B publishes.
    gossip_b
        .publish(&topic, b"Post from B".to_vec())
        .await
        .unwrap();

    // A receives B's post.
    let msg_on_a = tokio::time::timeout(Duration::from_secs(5), sub_a.recv())
        .await
        .expect("timeout: A waiting for B's post")
        .expect("A sub closed");
    assert_eq!(msg_on_a.payload, b"Post from B");

    transport_a.shutdown();
    transport_b.shutdown();
}

/// Test 4: Gossip fan-out through a chain of 3 nodes (A -> B -> C).
/// A posts something, C eventually gets it through B forwarding.
#[tokio::test]
async fn test_gossip_fan_out() {
    let (transport_a, id_a, addr_a) = make_node(50).await;
    let (transport_b, id_b, addr_b) = make_node(60).await;
    let (transport_c, id_c, _addr_c) = make_node(70).await;

    // B connects to A.
    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    // C connects to B.
    transport_c
        .connect(&PeerAddr {
            node_id: id_b,
            addresses: vec![addr_b.to_string()],
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Verify the chain: A knows B, B knows A and C, C knows B.
    assert!(transport_a.is_connected(&id_b), "A should know B");
    assert!(transport_b.is_connected(&id_a), "B should know A");
    assert!(transport_b.is_connected(&id_c), "B should know C");
    assert!(transport_c.is_connected(&id_b), "C should know B");
    // A does NOT directly know C.
    assert!(
        !transport_a.is_connected(&id_c),
        "A should NOT know C directly"
    );

    // Create gossip services.
    let gossip_a = EagerGossipService::new(id_a, Arc::clone(&transport_a));
    let gossip_b = EagerGossipService::new(id_b, Arc::clone(&transport_b));
    let gossip_c = EagerGossipService::new(id_c, Arc::clone(&transport_c));

    let topic = GossipTopic::public_feed();
    let _sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let _sub_b = gossip_b.subscribe(&topic).await.unwrap();
    let mut sub_c = gossip_c.subscribe(&topic).await.unwrap();

    // A publishes a message.
    gossip_a
        .publish(&topic, b"Hello from A, reaching C through B!".to_vec())
        .await
        .unwrap();

    // C should receive it (forwarded by B).
    let msg_on_c = tokio::time::timeout(Duration::from_secs(5), sub_c.recv())
        .await
        .expect("timeout: C waiting for A's message via B")
        .expect("C sub closed");

    assert_eq!(
        msg_on_c.payload, b"Hello from A, reaching C through B!",
        "C should see A's message forwarded through B"
    );
    assert_eq!(
        msg_on_c.source_node,
        *id_a.as_bytes(),
        "Origin should still be A even though B forwarded it"
    );

    transport_a.shutdown();
    transport_b.shutdown();
    transport_c.shutdown();
}
