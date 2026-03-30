//! Test: can two IrohTransport instances discover and connect via relay?

use ephemera_transport::{IrohTransport, PeerAddr, Transport};
use ephemera_types::NodeId;

#[tokio::test]
async fn test_two_iroh_transports_connect_via_relay() {
    // Create two transports with different deterministic keys.
    let key_a = [1u8; 32];
    let key_b = [2u8; 32];

    let transport_a = IrohTransport::with_secret_key(key_a)
        .await
        .expect("transport A should create");
    let transport_b = IrohTransport::with_secret_key(key_b)
        .await
        .expect("transport B should create");

    // Derive NodeIds from the Iroh endpoint public keys.
    let pubkey_a = transport_a.endpoint().id();
    let pubkey_b = transport_b.endpoint().id();
    let node_id_a = NodeId::from_bytes(*pubkey_a.as_bytes());
    let _node_id_b = NodeId::from_bytes(*pubkey_b.as_bytes());

    println!("Node A: {}", transport_a.node_id_hex());
    println!("Node B: {}", transport_b.node_id_hex());

    // Wait for relay connections to establish.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    // B connects to A by NodeId only (relay discovery, no address hints).
    let peer_addr = PeerAddr {
        node_id: node_id_a,
        addresses: vec![], // NO address hints — must use relay
    };

    println!("B connecting to A via relay...");
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        transport_b.connect(&peer_addr),
    )
    .await;

    match &result {
        Ok(Ok(())) => println!("SUCCESS: B connected to A!"),
        Ok(Err(e)) => println!("CONNECT FAILED: {e}"),
        Err(_) => println!("TIMEOUT: connect took more than 30 seconds"),
    }

    // Check peer counts.
    println!("A peers: {}", transport_a.connected_peers().len());
    println!("B peers: {}", transport_b.connected_peers().len());

    // Try sending a message if connected.
    if !transport_b.connected_peers().is_empty() {
        let peer = transport_b.connected_peers()[0];
        transport_b
            .send(&peer, b"hello from B")
            .await
            .expect("send should work");
        println!("B sent message to A");

        // Give A time to receive.
        if let Ok(Ok((sender, data))) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            transport_a.recv(),
        )
        .await
        {
            println!(
                "A received: {:?} from {:?}",
                String::from_utf8_lossy(&data),
                sender
            );
        } else {
            println!("A did not receive message");
        }
    }

    // Clean up.
    transport_a.shutdown().await;
    transport_b.shutdown().await;
}
