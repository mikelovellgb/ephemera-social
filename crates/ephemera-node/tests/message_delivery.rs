//! Integration tests for cross-network message delivery.
//!
//! Verifies that dead drop envelopes published on the `dm_delivery` gossip
//! topic are received and stored by remote nodes, closing the gap where
//! messages were previously written only to local SQLite.

use ephemera_config::NodeConfig;
use ephemera_events::EventBus;
use ephemera_gossip::topic::GossipTopic;
use ephemera_message::dead_drop::DEAD_DROP_MAX_TTL_SECS;
use ephemera_message::{DeadDropEnvelope, DeadDropService};
use ephemera_node::network::NetworkSubsystem;
use ephemera_node::services::ServiceContainer;
use ephemera_store::MetadataDb;
use ephemera_types::{IdentityKey, NodeId};

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Helper: create a ServiceContainer backed by a real temp directory.
fn make_services() -> (Arc<ServiceContainer>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let event_bus = EventBus::new();
    let svc = Arc::new(ServiceContainer::new(&config, event_bus).unwrap());
    (svc, dir)
}

/// Test that `MessageService::send()` publishes the dead drop envelope to the
/// `dm_delivery` gossip topic and a subscribing peer receives it.
#[tokio::test]
async fn test_message_reaches_remote_node() {
    // Set up two nodes with network subsystems.
    let id_a = NodeId::from_bytes([101; 32]);
    let id_b = NodeId::from_bytes([102; 32]);

    let net_a = NetworkSubsystem::new(id_a);
    let addr_a = net_a.start("127.0.0.1:0").await.unwrap();

    let net_b = NetworkSubsystem::new(id_b);
    let _addr_b = net_b.start("127.0.0.1:0").await.unwrap();

    // Connect B to A.
    net_b
        .connect_to_peer(&ephemera_transport::PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Both nodes subscribe to the dm_delivery topic.
    let dm_topic = GossipTopic::direct_messages();
    let _sub_a = net_a.subscribe(&dm_topic).await.unwrap();
    let mut sub_b = net_b.subscribe(&dm_topic).await.unwrap();

    // Create services for node A (the sender).
    let (svc_a, _dir_a) = make_services();
    svc_a.identity.create("test-pass-alice").await.unwrap();

    // Create a recipient identity (Bob).
    let bob_pubkey = IdentityKey::from_bytes([0xBB; 32]);
    let bob_hex = hex::encode(bob_pubkey.as_bytes());

    // Send a message from Alice to Bob via node A.
    let result = svc_a
        .messages
        .send(
            &bob_hex,
            "Hello Bob, this should cross the network!",
            Some(86400),
            &svc_a.identity,
            Some(&net_a),
            Some(&svc_a.dht_storage),
        )
        .await
        .unwrap();

    // Verify send result reports gossip publication.
    assert!(result["sent"].as_bool().unwrap());
    assert!(result["published_gossip"].as_bool().unwrap());
    assert!(result["dead_drop"].as_bool().unwrap());

    // Node B should receive the dead drop envelope via gossip.
    let received = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout: B did not receive the dead drop via gossip")
        .expect("B subscription channel closed");

    // Verify the payload deserializes to a valid DeadDropEnvelope.
    let envelope: DeadDropEnvelope = serde_json::from_slice(&received.payload)
        .expect("received gossip payload should be a valid DeadDropEnvelope");

    // The mailbox key should match Bob's pubkey-derived mailbox.
    let expected_mailbox = DeadDropService::mailbox_key(&bob_pubkey);
    assert_eq!(envelope.mailbox_key, *expected_mailbox.hash_bytes());

    // The sealed data is now E2E encrypted ciphertext (not plaintext).
    // Verify it's non-empty and NOT the plaintext (encryption is working).
    assert!(!envelope.sealed_data.is_empty(), "sealed data should not be empty");
    assert!(
        String::from_utf8(envelope.sealed_data.clone()).is_err()
            || !String::from_utf8(envelope.sealed_data.clone())
                .unwrap_or_default()
                .contains("Hello Bob"),
        "sealed data should NOT contain plaintext (it should be encrypted)"
    );

    net_a.shutdown().await;
    net_b.shutdown().await;
}

/// Test that the message ingest loop correctly stores dead drop envelopes
/// received from gossip into the local dead drop table.
#[tokio::test]
async fn test_message_ingest_stores_dead_drop() {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let metadata_dir = config.data_dir.join("metadata");
    std::fs::create_dir_all(&metadata_dir).unwrap();

    let metadata_db_path = config.metadata_db_path();
    let metadata_db = MetadataDb::open(&metadata_db_path).unwrap();
    let metadata_db_mutex = Mutex::new(metadata_db);

    let event_bus = EventBus::new();
    let mut event_rx = event_bus.subscribe();

    // Bob's identity (recipient).
    let bob_pubkey = IdentityKey::from_bytes([0xBB; 32]);
    let mailbox_key = DeadDropService::mailbox_key(&bob_pubkey);

    // Build a dead drop envelope as if Alice sent a message.
    let sealed_data = b"encrypted message content".to_vec();
    let msg_hash = blake3::hash(&sealed_data);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let envelope = DeadDropEnvelope {
        mailbox_key: *mailbox_key.hash_bytes(),
        message_id: *msg_hash.as_bytes(),
        sealed_data: sealed_data.clone(),
        deposited_at: now,
        expires_at: now + DEAD_DROP_MAX_TTL_SECS,
    };

    let envelope_bytes = serde_json::to_vec(&envelope).unwrap();

    // Set up a gossip subscription that we manually feed.
    let dm_topic = GossipTopic::direct_messages();
    let (sub, tx) = ephemera_gossip::TopicSubscription::new(dm_topic, 256);

    // Send the envelope into the subscription channel.
    let gossip_msg = ephemera_gossip::topic::GossipMessage {
        topic: dm_topic,
        payload: envelope_bytes,
        content_hash: *blake3::hash(&serde_json::to_vec(&envelope).unwrap()).as_bytes(),
        source_node: [0xAA; 32],
    };
    tx.send(gossip_msg).await.unwrap();

    // Drop the sender so the ingest loop will exit after processing.
    drop(tx);

    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Run the message ingest loop with Bob's identity.
    ephemera_node::message_ingest::message_ingest_loop(
        sub,
        metadata_db_mutex,
        event_bus.clone(),
        Some(bob_pubkey),
        shutdown_rx,
    )
    .await;

    // Verify the dead drop was stored in the metadata DB.
    let db2 = MetadataDb::open(&metadata_db_path).unwrap();
    let pending = DeadDropService::check_mailbox(&db2, &bob_pubkey).unwrap();
    assert_eq!(pending.len(), 1, "dead drop should be stored in local DB");
    assert_eq!(pending[0].sealed_envelope, sealed_data);

    // Verify a MessageReceived event was emitted (since it's addressed to us).
    let event = event_rx.try_recv();
    assert!(event.is_ok(), "a MessageReceived event should have been emitted");
    match event.unwrap() {
        ephemera_events::Event::MessageReceived { message_id, .. } => {
            assert_eq!(message_id, hex::encode(msg_hash.as_bytes()));
        }
        other => panic!("expected MessageReceived, got {:?}", other),
    }
}

/// Test end-to-end: Alice sends a message on node A, Bob receives it on node B
/// via the gossip network and the message ingest pipeline.
#[tokio::test]
async fn test_message_delivery_end_to_end() {
    // Set up two network nodes.
    let id_a = NodeId::from_bytes([201; 32]);
    let id_b = NodeId::from_bytes([202; 32]);

    let net_a = NetworkSubsystem::new(id_a);
    let addr_a = net_a.start("127.0.0.1:0").await.unwrap();

    let net_b = NetworkSubsystem::new(id_b);
    let _addr_b = net_b.start("127.0.0.1:0").await.unwrap();

    // Connect B to A.
    net_b
        .connect_to_peer(&ephemera_transport::PeerAddr {
            node_id: id_a,
            addresses: vec![addr_a.to_string()],
        })
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Subscribe node A to dm_delivery (so publish works).
    let dm_topic = GossipTopic::direct_messages();
    let _sub_a = net_a.subscribe(&dm_topic).await.unwrap();

    // Subscribe node B to dm_delivery via the ingest loop.
    let sub_b = net_b.subscribe(&dm_topic).await.unwrap();

    // Create services for Alice (node A).
    let (svc_a, _dir_a) = make_services();
    svc_a.identity.create("alice-pass").await.unwrap();

    // Create a DB for Bob (node B) to receive the dead drop.
    let dir_b = tempfile::tempdir().unwrap();
    let config_b = NodeConfig::default_for(dir_b.path());
    let metadata_dir_b = config_b.data_dir.join("metadata");
    std::fs::create_dir_all(&metadata_dir_b).unwrap();
    let db_path_b = config_b.metadata_db_path();
    let db_b = MetadataDb::open(&db_path_b).unwrap();
    let db_b_mutex = Mutex::new(db_b);

    let event_bus_b = EventBus::new();
    let mut event_rx_b = event_bus_b.subscribe();

    // Bob's identity.
    let bob_pubkey = IdentityKey::from_bytes([0xBB; 32]);
    let bob_hex = hex::encode(bob_pubkey.as_bytes());

    // Spawn the message ingest loop for node B.
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let shutdown_rx_clone = shutdown_rx;
    let ingest_handle = tokio::spawn(async move {
        ephemera_node::message_ingest::message_ingest_loop(
            sub_b,
            db_b_mutex,
            event_bus_b,
            Some(bob_pubkey),
            shutdown_rx_clone,
        )
        .await;
    });

    // Alice sends a message to Bob.
    let result = svc_a
        .messages
        .send(
            &bob_hex,
            "End-to-end test message from Alice to Bob!",
            Some(86400),
            &svc_a.identity,
            Some(&net_a),
            None,
        )
        .await
        .unwrap();

    assert!(result["sent"].as_bool().unwrap());
    assert!(result["published_gossip"].as_bool().unwrap());

    // Wait for the message to propagate through gossip and be ingested.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify Bob received a MessageReceived event.
    let event = event_rx_b.try_recv();
    assert!(event.is_ok(), "Bob should have received a MessageReceived event");

    // Verify the dead drop is in Bob's local DB.
    let db_b2 = MetadataDb::open(&db_path_b).unwrap();
    let pending = DeadDropService::check_mailbox(&db_b2, &bob_pubkey).unwrap();
    assert_eq!(pending.len(), 1, "Bob's dead drop should contain the message");

    // sealed_envelope is now E2E encrypted ciphertext.
    assert!(
        !pending[0].sealed_envelope.is_empty(),
        "sealed envelope should not be empty"
    );

    // Cleanup.
    _shutdown_tx.send(true).unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), ingest_handle).await;
    net_a.shutdown().await;
    net_b.shutdown().await;
}

/// Test that dead drop envelopes are stored in the DHT and can be
/// retrieved by the polling loop.
#[tokio::test]
async fn test_offline_message_via_dht() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();

    let bob_pubkey = IdentityKey::from_bytes([0xBB; 32]);
    let bob_hex = hex::encode(bob_pubkey.as_bytes());

    // Send a message with DHT storage only (no network).
    let result = svc
        .messages
        .send(
            &bob_hex,
            "offline message for Bob via DHT",
            Some(86400),
            &svc.identity,
            None,
            Some(&svc.dht_storage),
        )
        .await
        .unwrap();

    assert!(result["sent"].as_bool().unwrap());
    assert!(result["published_dht"].as_bool().unwrap());
    assert!(!result["published_gossip"].as_bool().unwrap());

    // Verify the DHT contains the dead drop record.
    let mailbox_key = DeadDropService::mailbox_key(&bob_pubkey);
    let dht_key = *mailbox_key.hash_bytes();
    let dht_storage = svc.dht_storage.lock().unwrap();
    let record = dht_storage.get(&dht_key);
    assert!(record.is_some(), "DHT should contain the dead drop record");

    // Verify the record value deserializes to a DeadDropEnvelope.
    let envelope: DeadDropEnvelope =
        serde_json::from_slice(&record.unwrap().value).unwrap();
    assert_eq!(envelope.mailbox_key, *mailbox_key.hash_bytes());

    // sealed_data is now E2E encrypted ciphertext, not plaintext.
    assert!(
        !envelope.sealed_data.is_empty(),
        "sealed data should not be empty (contains encrypted message)"
    );
}
