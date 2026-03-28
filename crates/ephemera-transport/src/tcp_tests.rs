use super::*;
use crate::{PeerAddr, Transport};
use ephemera_types::NodeId;

#[tokio::test]
async fn listen_and_connect() {
    let node_a_id = NodeId::from_bytes([1; 32]);
    let node_b_id = NodeId::from_bytes([2; 32]);

    let transport_a = TcpTransport::new(node_a_id);
    let addr = transport_a.listen("127.0.0.1:0").await.unwrap();

    let transport_b = TcpTransport::new(node_b_id);
    transport_b
        .connect(&PeerAddr {
            node_id: node_a_id,
            addresses: vec![addr.to_string()],
        })
        .await
        .unwrap();

    // Give the listener time to accept and set up.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // B should be connected to A, and A should have B.
    assert!(transport_b.is_connected(&node_a_id));
    assert!(transport_a.is_connected(&node_b_id));

    transport_a.shutdown();
}

#[tokio::test]
async fn send_and_recv() {
    let node_a_id = NodeId::from_bytes([10; 32]);
    let node_b_id = NodeId::from_bytes([20; 32]);

    let transport_a = TcpTransport::new(node_a_id);
    let addr = transport_a.listen("127.0.0.1:0").await.unwrap();

    let transport_b = TcpTransport::new(node_b_id);
    transport_b
        .connect(&PeerAddr {
            node_id: node_a_id,
            addresses: vec![addr.to_string()],
        })
        .await
        .unwrap();

    // Wait for handshake completion.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // B sends to A.
    let payload = b"hello from B";
    transport_b.send(&node_a_id, payload).await.unwrap();

    // A receives from B.
    let (sender, data) = transport_a.recv().await.unwrap();
    assert_eq!(sender, node_b_id);
    assert_eq!(data, payload);

    transport_a.shutdown();
}

#[tokio::test]
async fn bidirectional_send() {
    let id_a = NodeId::from_bytes([30; 32]);
    let id_b = NodeId::from_bytes([40; 32]);

    let t_a = TcpTransport::new(id_a);
    let addr = t_a.listen("127.0.0.1:0").await.unwrap();

    let t_b = TcpTransport::new(id_b);
    t_b.connect(&PeerAddr {
        node_id: id_a,
        addresses: vec![addr.to_string()],
    })
    .await
    .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // B -> A
    t_b.send(&id_a, b"from B").await.unwrap();
    let (sender, data) = t_a.recv().await.unwrap();
    assert_eq!(sender, id_b);
    assert_eq!(data, b"from B");

    // A -> B
    t_a.send(&id_b, b"from A").await.unwrap();
    let (sender, data) = t_b.recv().await.unwrap();
    assert_eq!(sender, id_a);
    assert_eq!(data, b"from A");

    t_a.shutdown();
}
