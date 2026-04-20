use super::*;

/// A minimal mock transport for testing gossip locally.
/// Supports local subscribe/publish only — no real peer connections.
struct MockTransport {
    id: NodeId,
    peers: std::sync::Mutex<Vec<NodeId>>,
    inbound: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<(NodeId, Vec<u8>)>>,
    inbound_tx: tokio::sync::mpsc::Sender<(NodeId, Vec<u8>)>,
}

impl MockTransport {
    fn new(id: NodeId) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        Self {
            id,
            peers: std::sync::Mutex::new(Vec::new()),
            inbound: tokio::sync::Mutex::new(rx),
            inbound_tx: tx,
        }
    }
}

#[async_trait::async_trait]
impl ephemera_transport::Transport for MockTransport {
    async fn send(&self, _peer: &NodeId, _data: &[u8]) -> Result<(), ephemera_transport::TransportError> {
        Ok(())
    }
    async fn recv(&self) -> Result<(NodeId, Vec<u8>), ephemera_transport::TransportError> {
        let mut rx = self.inbound.lock().await;
        rx.recv().await.ok_or(ephemera_transport::TransportError::ConnectionClosed {
            reason: "mock channel closed".into(),
        })
    }
    async fn connect(&self, addr: &ephemera_transport::PeerAddr) -> Result<(), ephemera_transport::TransportError> {
        self.peers.lock().unwrap().push(addr.node_id);
        Ok(())
    }
    async fn disconnect(&self, peer: &NodeId) -> Result<(), ephemera_transport::TransportError> {
        self.peers.lock().unwrap().retain(|p| p != peer);
        Ok(())
    }
    fn connected_peers(&self) -> Vec<NodeId> {
        self.peers.lock().unwrap().clone()
    }
    fn is_connected(&self, peer: &NodeId) -> bool {
        self.peers.lock().unwrap().contains(peer)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[tokio::test]
async fn publish_and_receive_locally() {
    let id = NodeId::from_bytes([1; 32]);
    let transport = Arc::new(MockTransport::new(id));

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
