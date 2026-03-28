use super::*;
use ephemera_transport::tcp::TcpTransport;
use ephemera_transport::PeerAddr;

#[tokio::test]
async fn publish_and_receive_locally() {
    let id = NodeId::from_bytes([1; 32]);
    let transport = Arc::new(TcpTransport::new(id));
    let _ = transport.listen("127.0.0.1:0").await.unwrap();

    let gossip = EagerGossipService::new(id, transport);
    let topic = GossipTopic::public_feed();
    let mut sub = gossip.subscribe(&topic).await.unwrap();

    gossip
        .publish(&topic, b"hello local".to_vec())
        .await
        .unwrap();

    let msg = sub.recv().await.unwrap();
    assert_eq!(msg.payload, b"hello local");
}

#[tokio::test]
async fn gossip_between_two_nodes() {
    let id_a = NodeId::from_bytes([10; 32]);
    let id_b = NodeId::from_bytes([20; 32]);

    let transport_a = Arc::new(TcpTransport::new(id_a));
    let addr_a = transport_a.listen("127.0.0.1:0").await.unwrap();

    let transport_b = Arc::new(TcpTransport::new(id_b));
    let _ = transport_b.listen("127.0.0.1:0").await.unwrap();

    // B connects to A.
    transport_b
        .connect(&PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let gossip_a = EagerGossipService::new(id_a, Arc::clone(&transport_a));
    let gossip_b = EagerGossipService::new(id_b, Arc::clone(&transport_b));

    let topic = GossipTopic::public_feed();
    let _sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let mut sub_b = gossip_b.subscribe(&topic).await.unwrap();

    // A publishes.
    gossip_a
        .publish(&topic, b"from node A".to_vec())
        .await
        .unwrap();

    // B should receive it.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(2), sub_b.recv())
        .await
        .expect("timeout waiting for gossip message")
        .expect("channel closed");

    assert_eq!(msg.payload, b"from node A");

    transport_a.shutdown();
}

/// Test the PlumTree lazy-push IHAVE/IWANT flow.
///
/// Sets up 3 nodes: A <-> B <-> C (no direct A-C link).
/// Configures fanout with eager_push_peers=1, lazy_push_peers=10.
/// When A publishes, B should get a full message eagerly AND C should
/// either get a full message (if B is the eager peer) or get an IHAVE
/// followed by IWANT flow. Either way, C should receive the content.
#[tokio::test]
async fn test_lazy_push_reduces_messages() {
    use crate::fanout::FanoutConfig;
    use std::time::Duration;

    let id_a = NodeId::from_bytes([41; 32]);
    let id_b = NodeId::from_bytes([42; 32]);
    let id_c = NodeId::from_bytes([43; 32]);

    let transport_a = Arc::new(TcpTransport::new(id_a));
    let addr_a = transport_a.listen("127.0.0.1:0").await.unwrap();

    let transport_b = Arc::new(TcpTransport::new(id_b));
    let addr_b = transport_b.listen("127.0.0.1:0").await.unwrap();

    let transport_c = Arc::new(TcpTransport::new(id_c));
    let _ = transport_c.listen("127.0.0.1:0").await.unwrap();

    // B connects to A; C connects to B.
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

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Use lazy timeout of 500ms so IWANT fires quickly in tests.
    let fanout = FanoutConfig {
        eager_push_peers: 1,
        lazy_push_peers: 10,
        lazy_timeout: Duration::from_millis(500),
        ..FanoutConfig::default()
    };

    let gossip_a = EagerGossipService::with_fanout(id_a, Arc::clone(&transport_a), fanout.clone());
    let gossip_b = EagerGossipService::with_fanout(id_b, Arc::clone(&transport_b), fanout.clone());
    let gossip_c = EagerGossipService::with_fanout(id_c, Arc::clone(&transport_c), fanout);

    let topic = GossipTopic::public_feed();
    let _sub_a = gossip_a.subscribe(&topic).await.unwrap();
    let _sub_b = gossip_b.subscribe(&topic).await.unwrap();
    let mut sub_c = gossip_c.subscribe(&topic).await.unwrap();

    // A publishes a message.
    gossip_a
        .publish(&topic, b"plumtree test message".to_vec())
        .await
        .unwrap();

    // C should eventually receive the content, either via:
    // 1. Eager push from B (if B got the full message and C is B's eager peer)
    // 2. IHAVE -> IWANT flow (lazy pull)
    // We wait long enough for the lazy timeout + IWANT round trip.
    let received = tokio::time::timeout(Duration::from_secs(10), sub_c.recv())
        .await
        .expect("timeout: C did not receive the message via PlumTree flow")
        .expect("C subscription channel closed");

    assert_eq!(received.payload, b"plumtree test message");

    transport_a.shutdown();
    transport_b.shutdown();
    transport_c.shutdown();
}

/// Test that the IHAVE/IWANT envelope serialization round-trips correctly.
#[test]
fn envelope_serialization_round_trip() {
    let full = GossipEnvelope::FullMessage(GossipWireMessage {
        topic: [0xAA; 32],
        payload: vec![1, 2, 3],
        content_hash: [0xBB; 32],
        origin: [0xCC; 32],
    });
    let bytes = serde_json::to_vec(&full).unwrap();
    let decoded: GossipEnvelope = serde_json::from_slice(&bytes).unwrap();
    match decoded {
        GossipEnvelope::FullMessage(msg) => {
            assert_eq!(msg.payload, vec![1, 2, 3]);
            assert_eq!(msg.content_hash, [0xBB; 32]);
        }
        _ => panic!("expected FullMessage"),
    }

    let ihave = GossipEnvelope::IHave(IHaveMessage {
        topic: [0xDD; 32],
        content_hash: [0xEE; 32],
        origin: [0xFF; 32],
    });
    let bytes = serde_json::to_vec(&ihave).unwrap();
    let decoded: GossipEnvelope = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(decoded, GossipEnvelope::IHave(_)));

    let iwant = GossipEnvelope::IWant(IWantMessage {
        content_hash: [0x11; 32],
    });
    let bytes = serde_json::to_vec(&iwant).unwrap();
    let decoded: GossipEnvelope = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(decoded, GossipEnvelope::IWant(_)));
}
