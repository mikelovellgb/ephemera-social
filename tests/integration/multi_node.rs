//! Multi-node integration tests for the Ephemera platform.
//!
//! These tests spin up real TCP transports and gossip services to prove
//! that posts propagate across a network of nodes. They exercise:
//!
//! - Two-node post propagation
//! - Three-node gossip chain (A -> B -> C with no direct A-C link)
//! - Bidirectional post exchange
//! - Video/media post distribution
//! - Content expiry across the network
//! - Node resilience (disconnect/reconnect)
//!
//! Run with: `cargo test -p ephemera-node --test multi_node`

use ephemera_gossip::service::EagerGossipService;
use ephemera_gossip::topic::GossipTopic;
use ephemera_node::network::NetworkSubsystem;
use ephemera_transport::tcp::TcpTransport;
use ephemera_transport::{PeerAddr, Transport};
use ephemera_types::NodeId;

use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Create a `NetworkSubsystem` with a unique node ID and start it on a
/// random port. Returns the subsystem and the bound address.
async fn start_node(id_byte: u8) -> (NetworkSubsystem, std::net::SocketAddr) {
    let id = NodeId::from_bytes([id_byte; 32]);
    let net = NetworkSubsystem::new(id);
    let addr = net
        .start("127.0.0.1:0")
        .await
        .expect("network subsystem should start");
    (net, addr)
}

/// Connect `source` to `target` via TCP, using the target's listen address.
async fn connect_nodes(
    source: &NetworkSubsystem,
    target_id: NodeId,
    target_addr: std::net::SocketAddr,
) {
    source
        .connect_to_peer(&PeerAddr {
            node_id: target_id,
            addresses: vec![target_addr.to_string()],
        })
        .await
        .expect("connect_to_peer should succeed");
}

/// Wait briefly for the TCP handshake to complete on both sides.
async fn wait_for_handshake() {
    tokio::time::sleep(Duration::from_millis(250)).await;
}

/// Collect up to `n` messages from a topic subscription, waiting up to
/// 5 seconds per message. Returns the payloads received.
async fn collect_n_messages(
    sub: &mut ephemera_gossip::TopicSubscription,
    n: usize,
) -> Vec<Vec<u8>> {
    let mut messages = Vec::with_capacity(n);
    for _ in 0..n {
        match tokio::time::timeout(Duration::from_secs(5), sub.recv()).await {
            Ok(Some(msg)) => messages.push(msg.payload),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    messages
}

/// Create a low-level transport + gossip pair (without `NetworkSubsystem`)
/// for scenarios that need finer control.
async fn make_gossip_node(
    id_byte: u8,
) -> (
    Arc<TcpTransport>,
    NodeId,
    std::net::SocketAddr,
    EagerGossipService,
) {
    let id = NodeId::from_bytes([id_byte; 32]);
    let transport = Arc::new(TcpTransport::new(id));
    let addr = transport
        .listen("127.0.0.1:0")
        .await
        .expect("transport should listen");
    let gossip = EagerGossipService::new(id, Arc::clone(&transport));
    (transport, id, addr, gossip)
}

// ---------------------------------------------------------------------------
// Scenario 1: Two-Node Post Propagation
// ---------------------------------------------------------------------------

/// Start Node A and Node B on different ports, connect them, have A
/// publish a post, and verify B receives it with matching content.
#[tokio::test]
async fn scenario_1_two_node_post_propagation() {
    let (net_a, addr_a) = start_node(1).await;
    let (net_b, _addr_b) = start_node(2).await;

    // Connect B -> A.
    connect_nodes(&net_b, *net_a.local_id(), addr_a).await;
    wait_for_handshake().await;

    // Verify both sides see the connection.
    assert_eq!(net_a.peer_count(), 1, "Node A should have 1 connected peer");
    assert_eq!(net_b.peer_count(), 1, "Node B should have 1 connected peer");

    // Subscribe both nodes to the public feed.
    let _sub_a = net_a.subscribe_public_feed().await.unwrap();
    let mut sub_b = net_b.subscribe_public_feed().await.unwrap();

    // Node A creates a text post.
    let post_body = b"Hello from Node A! This is a test post.".to_vec();
    net_a.publish_post(post_body.clone()).await.unwrap();

    // Wait up to 5 seconds for Node B to receive the post.
    let received = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout: Node B did not receive the post within 5 seconds")
        .expect("subscription channel closed unexpectedly");

    // Verify content matches exactly.
    assert_eq!(
        received.payload, post_body,
        "Node B should receive exactly the content from Node A"
    );
    assert_eq!(
        received.source_node,
        *net_a.local_id().as_bytes(),
        "Origin should be Node A"
    );

    net_a.shutdown().await;
    net_b.shutdown().await;
}

// ---------------------------------------------------------------------------
// Scenario 2: Three-Node Gossip Chain
// ---------------------------------------------------------------------------

/// Start A, B, C. Connect A <-> B and B <-> C (A and C are NOT directly
/// connected). Node A publishes a post and we verify that Node C receives
/// it via B's forwarding within 10 seconds.
#[tokio::test]
async fn scenario_2_three_node_gossip_chain() {
    // Use distinct id bytes to avoid collisions with other tests.
    let (transport_a, id_a, addr_a, gossip_a) = make_gossip_node(10).await;
    let (transport_b, id_b, addr_b, gossip_b) = make_gossip_node(20).await;
    let (transport_c, id_c, _addr_c, gossip_c) = make_gossip_node(30).await;

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

    wait_for_handshake().await;

    // Verify the chain topology.
    assert!(transport_a.is_connected(&id_b), "A should know B");
    assert!(transport_b.is_connected(&id_a), "B should know A");
    assert!(transport_b.is_connected(&id_c), "B should know C");
    assert!(transport_c.is_connected(&id_b), "C should know B");
    assert!(
        !transport_a.is_connected(&id_c),
        "A should NOT directly know C"
    );
    assert!(
        !transport_c.is_connected(&id_a),
        "C should NOT directly know A"
    );

    // Subscribe all three nodes to the public feed.
    let topic = GossipTopic::public_feed();
    let _sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let _sub_b = gossip_b.subscribe(&topic).await.unwrap();
    let mut sub_c = gossip_c.subscribe(&topic).await.unwrap();

    // Node A publishes a post.
    let post_content = b"Gossip chain test: A -> B -> C".to_vec();
    gossip_a
        .publish(&topic, post_content.clone())
        .await
        .unwrap();

    // Wait up to 10 seconds for Node C to receive the post (via B's forwarding).
    let received = tokio::time::timeout(Duration::from_secs(10), sub_c.recv())
        .await
        .expect("timeout: Node C did not receive A's post via B within 10 seconds")
        .expect("C subscription channel closed unexpectedly");

    assert_eq!(
        received.payload, post_content,
        "Node C should receive the exact content from Node A"
    );
    assert_eq!(
        received.source_node,
        *id_a.as_bytes(),
        "Origin should be Node A even though B forwarded it"
    );

    transport_a.shutdown();
    transport_b.shutdown();
    transport_c.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 3: Bidirectional Post Exchange
// ---------------------------------------------------------------------------

/// Start A and B, connect them. A posts "hello from A", B posts
/// "hello from B". Verify both nodes see both posts.
#[tokio::test]
async fn scenario_3_bidirectional_post_exchange() {
    let (net_a, addr_a) = start_node(40).await;
    let (net_b, _addr_b) = start_node(50).await;

    connect_nodes(&net_b, *net_a.local_id(), addr_a).await;
    wait_for_handshake().await;

    // Subscribe both.
    let mut sub_a = net_a.subscribe_public_feed().await.unwrap();
    let mut sub_b = net_b.subscribe_public_feed().await.unwrap();

    // A posts.
    let msg_a = b"hello from A".to_vec();
    net_a.publish_post(msg_a.clone()).await.unwrap();

    // B receives A's post.
    let on_b = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout: B waiting for A's post")
        .expect("B sub closed");
    assert_eq!(on_b.payload, msg_a, "B should see A's post");

    // A also sees its own post (local delivery on publish).
    let on_a_local = tokio::time::timeout(Duration::from_secs(2), sub_a.recv())
        .await
        .expect("timeout: A waiting for local echo of its own post")
        .expect("A sub closed");
    assert_eq!(
        on_a_local.payload, msg_a,
        "A should see its own post locally"
    );

    // B posts.
    let msg_b = b"hello from B".to_vec();
    net_b.publish_post(msg_b.clone()).await.unwrap();

    // A receives B's post.
    let on_a = tokio::time::timeout(Duration::from_secs(5), sub_a.recv())
        .await
        .expect("timeout: A waiting for B's post")
        .expect("A sub closed");
    assert_eq!(on_a.payload, msg_b, "A should see B's post");

    // B also sees its own post locally.
    let on_b_local = tokio::time::timeout(Duration::from_secs(2), sub_b.recv())
        .await
        .expect("timeout: B waiting for local echo of its own post")
        .expect("B sub closed");
    assert_eq!(
        on_b_local.payload, msg_b,
        "B should see its own post locally"
    );

    net_a.shutdown().await;
    net_b.shutdown().await;
}

// ---------------------------------------------------------------------------
// Scenario 4: Video Post Distribution
// ---------------------------------------------------------------------------

/// Start A and B, connect them. A creates a post with video-like data
/// (ftyp header + random payload simulating MP4 chunks). Verify B receives
/// the post metadata and the full video payload.
#[tokio::test]
async fn scenario_4_video_post_distribution() {
    let (net_a, addr_a) = start_node(60).await;
    let (net_b, _addr_b) = start_node(70).await;

    connect_nodes(&net_b, *net_a.local_id(), addr_a).await;
    wait_for_handshake().await;

    let _sub_a = net_a.subscribe_public_feed().await.unwrap();
    let mut sub_b = net_b.subscribe_public_feed().await.unwrap();

    // Build a video-like payload: ftyp header + random "chunk" data.
    // This simulates a serialized post envelope containing video metadata
    // and chunk references.
    let mut video_payload = Vec::with_capacity(512);
    // Minimal ftyp box header (20 bytes).
    video_payload.extend_from_slice(&[
        0x00, 0x00, 0x00, 0x14, // box size = 20
        0x66, 0x74, 0x79, 0x70, // "ftyp"
        0x69, 0x73, 0x6F, 0x6D, // "isom"
        0x00, 0x00, 0x02, 0x00, // minor version
        0x69, 0x73, 0x6F, 0x6D, // compatible brand "isom"
    ]);
    // Append random "video chunk" data.
    for i in 0u8..200 {
        video_payload.push(i);
    }

    // Publish as a raw gossip message (in a real node, this would be a
    // serialized PostEnvelope with media_type = "video/mp4").
    net_a.publish_post(video_payload.clone()).await.unwrap();

    // B should receive the full payload.
    let received = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout: B did not receive the video post within 5 seconds")
        .expect("B sub closed");

    // Verify the video payload arrived intact.
    assert_eq!(
        received.payload.len(),
        video_payload.len(),
        "video payload length should match"
    );
    assert_eq!(
        &received.payload[4..8],
        b"ftyp",
        "B should see the ftyp header in the received video data"
    );
    assert_eq!(
        received.payload, video_payload,
        "the full video payload should be byte-for-byte identical"
    );

    net_a.shutdown().await;
    net_b.shutdown().await;
}

// ---------------------------------------------------------------------------
// Scenario 5: Content Expiry Across Network
// ---------------------------------------------------------------------------

/// Start A and B, connect them. A creates a post that propagates to B.
/// Both sides see it. Then we verify that the gossip dedup set prevents
/// re-delivery (simulating the "post is already known" aspect of expiry).
///
/// Note: True time-based TTL expiry with GC requires the full
/// `EphemeraNode` stack and backdating `expires_at` in SQLite (as done
/// in `core_loop.rs`). At the transport/gossip layer, "expiry" manifests
/// as the dedup set preventing duplicate delivery. This test verifies
/// that behavior.
#[tokio::test]
async fn scenario_5_content_expiry_dedup() {
    let (transport_a, id_a, addr_a, gossip_a) = make_gossip_node(80).await;
    let (transport_b, _id_b, _addr_b, gossip_b) = make_gossip_node(90).await;

    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    wait_for_handshake().await;

    let topic = GossipTopic::public_feed();
    let _sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let mut sub_b = gossip_b.subscribe(&topic).await.unwrap();

    // A publishes a post.
    let short_lived = b"This post has a short TTL (simulated)".to_vec();
    gossip_a.publish(&topic, short_lived.clone()).await.unwrap();

    // B receives it.
    let received = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout: B did not receive the post")
        .expect("B sub closed");
    assert_eq!(received.payload, short_lived);

    // If A re-publishes the SAME content, B should NOT receive it again
    // because the gossip dedup set already contains this content hash.
    gossip_a.publish(&topic, short_lived.clone()).await.unwrap();

    // Wait briefly -- B should NOT get a second copy.
    let duplicate = tokio::time::timeout(Duration::from_millis(500), sub_b.recv()).await;
    assert!(
        duplicate.is_err(),
        "B should NOT receive a duplicate of the same content (dedup should filter it)"
    );

    transport_a.shutdown();
    transport_b.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 6: Node Resilience
// ---------------------------------------------------------------------------

/// Start A, B, C in a chain (A <-> B <-> C). A posts something and C
/// receives it via B. Then B disconnects. A posts something new and C
/// does NOT receive it (B is down). Finally, we verify that B's
/// disconnection is reflected in the peer counts.
#[tokio::test]
async fn scenario_6_node_resilience() {
    let (transport_a, id_a, addr_a, gossip_a) = make_gossip_node(100).await;
    let (transport_b, id_b, addr_b, gossip_b) = make_gossip_node(110).await;
    let (transport_c, id_c, _addr_c, gossip_c) = make_gossip_node(120).await;

    // Build chain: B -> A, C -> B.
    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    transport_c
        .connect(&PeerAddr {
            node_id: id_b,
            addresses: vec![addr_b.to_string()],
        })
        .await
        .unwrap();

    wait_for_handshake().await;

    // Verify the chain.
    assert!(transport_a.is_connected(&id_b), "A should know B");
    assert!(transport_b.is_connected(&id_c), "B should know C");
    assert!(transport_c.is_connected(&id_b), "C should know B");

    let topic = GossipTopic::public_feed();
    let _sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let _sub_b = gossip_b.subscribe(&topic).await.unwrap();
    let mut sub_c = gossip_c.subscribe(&topic).await.unwrap();

    // A posts -- C should receive it via B.
    gossip_a
        .publish(&topic, b"Message before disconnect".to_vec())
        .await
        .unwrap();

    let pre_disconnect = tokio::time::timeout(Duration::from_secs(5), sub_c.recv())
        .await
        .expect("timeout: C did not receive A's pre-disconnect post")
        .expect("C sub closed");
    assert_eq!(pre_disconnect.payload, b"Message before disconnect");

    // Disconnect B from both A and C (simulating B going offline).
    transport_b.disconnect(&id_a).await.unwrap();
    transport_b.disconnect(&id_c).await.unwrap();

    // Give the reader/writer tasks time to notice the disconnection.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A posts something new -- C should NOT receive it because B is down.
    gossip_a
        .publish(&topic, b"Message after disconnect".to_vec())
        .await
        .unwrap();

    // Wait briefly -- C should NOT get the new message.
    let post_disconnect = tokio::time::timeout(Duration::from_millis(1500), sub_c.recv()).await;
    assert!(
        post_disconnect.is_err(),
        "C should NOT receive A's post after B disconnects"
    );

    // Verify that B no longer has peers (both directions were disconnected).
    // Note: the peer map may have been cleaned up by reader/writer tasks.
    assert!(
        !transport_b.is_connected(&id_a),
        "B should no longer know A after disconnect"
    );
    assert!(
        !transport_b.is_connected(&id_c),
        "B should no longer know C after disconnect"
    );

    transport_a.shutdown();
    transport_b.shutdown();
    transport_c.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 7: Multiple Topics Isolation
// ---------------------------------------------------------------------------

/// Verify that messages published on one gossip topic do NOT leak to
/// subscribers of a different topic.
#[tokio::test]
async fn scenario_7_topic_isolation() {
    let (transport_a, id_a, addr_a, gossip_a) = make_gossip_node(130).await;
    let (transport_b, _id_b, _addr_b, gossip_b) = make_gossip_node(140).await;

    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    wait_for_handshake().await;

    let topic_public = GossipTopic::public_feed();
    let topic_mod = GossipTopic::moderation();

    // A subscribes to public feed, B subscribes to moderation.
    let _sub_a_public = gossip_a.subscribe(&topic_public).await.unwrap();
    let mut sub_b_mod = gossip_b.subscribe(&topic_mod).await.unwrap();
    let mut sub_b_public = gossip_b.subscribe(&topic_public).await.unwrap();

    // A publishes on the public feed topic.
    gossip_a
        .publish(&topic_public, b"public post".to_vec())
        .await
        .unwrap();

    // B should receive it on the public feed subscription.
    let public_msg = tokio::time::timeout(Duration::from_secs(5), sub_b_public.recv())
        .await
        .expect("timeout: B did not receive public post")
        .expect("B public sub closed");
    assert_eq!(public_msg.payload, b"public post");

    // B should NOT receive it on the moderation subscription.
    let mod_msg = tokio::time::timeout(Duration::from_millis(500), sub_b_mod.recv()).await;
    assert!(
        mod_msg.is_err(),
        "B's moderation subscription should NOT receive a public feed message"
    );

    transport_a.shutdown();
    transport_b.shutdown();
}

// ---------------------------------------------------------------------------
// Scenario 8: Concurrent Multi-Publisher
// ---------------------------------------------------------------------------

/// Three nodes in full mesh. Each publishes a unique message. All three
/// should eventually see all three messages.
#[tokio::test]
async fn scenario_8_concurrent_multi_publisher() {
    let (transport_a, id_a, addr_a, gossip_a) = make_gossip_node(150).await;
    let (transport_b, id_b, addr_b, gossip_b) = make_gossip_node(160).await;
    let (transport_c, _id_c, _addr_c, gossip_c) = make_gossip_node(170).await;

    // Full mesh: B -> A, C -> A, C -> B.
    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();
    transport_c
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();
    transport_c
        .connect(&PeerAddr {
            node_id: id_b,
            addresses: vec![addr_b.to_string()],
        })
        .await
        .unwrap();

    wait_for_handshake().await;

    let topic = GossipTopic::public_feed();
    let mut sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let mut sub_b = gossip_b.subscribe(&topic).await.unwrap();
    let mut sub_c = gossip_c.subscribe(&topic).await.unwrap();

    // Each node publishes a unique message.
    gossip_a.publish(&topic, b"from A".to_vec()).await.unwrap();
    gossip_b.publish(&topic, b"from B".to_vec()).await.unwrap();
    gossip_c.publish(&topic, b"from C".to_vec()).await.unwrap();

    // Collect messages on each node.
    // Each node should see:
    //   - its own message (local delivery)
    //   - the other two nodes' messages (remote delivery)
    // = 3 messages per node.

    let msgs_a = collect_n_messages(&mut sub_a, 3).await;
    let msgs_b = collect_n_messages(&mut sub_b, 3).await;
    let msgs_c = collect_n_messages(&mut sub_c, 3).await;

    // Each node should have received exactly 3 messages.
    assert_eq!(msgs_a.len(), 3, "Node A should have 3 messages");
    assert_eq!(msgs_b.len(), 3, "Node B should have 3 messages");
    assert_eq!(msgs_c.len(), 3, "Node C should have 3 messages");

    // Verify all three payloads are present on each node.
    for (label, msgs) in [("A", &msgs_a), ("B", &msgs_b), ("C", &msgs_c)] {
        assert!(
            msgs.iter().any(|m| m == b"from A"),
            "Node {label} should have A's message"
        );
        assert!(
            msgs.iter().any(|m| m == b"from B"),
            "Node {label} should have B's message"
        );
        assert!(
            msgs.iter().any(|m| m == b"from C"),
            "Node {label} should have C's message"
        );
    }

    transport_a.shutdown();
    transport_b.shutdown();
    transport_c.shutdown();
}
