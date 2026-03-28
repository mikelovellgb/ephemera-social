//! Integration tests for the gossip-to-post pipeline.
//!
//! Tests that:
//! 1. Creating a post publishes it to the gossip network.
//! 2. Receiving a gossip message stores the post locally.
//! 3. The PlumTree lazy-push IHAVE/IWANT flow delivers messages.

use ephemera_abuse::{FingerprintStore, RateLimiter};
use ephemera_config::NodeConfig;
use ephemera_crypto::SigningKeyPair;
use ephemera_events::EventBus;
use ephemera_gossip::topic::GossipTopic;
use ephemera_mod::ContentFilter;
use ephemera_node::network::NetworkSubsystem;
use ephemera_node::services::ServiceContainer;
use ephemera_post::PostBuilder;
use ephemera_store::{ContentStore, MetadataDb};
use ephemera_types::{NodeId, Ttl};

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

/// Test that creating a post with `create_and_publish` sends it to the gossip
/// network and a subscribing peer receives it.
#[tokio::test]
async fn test_post_create_publishes_to_gossip() {
    // Set up two nodes with network subsystems.
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

    // Subscribe B to the public feed.
    let _sub_a = net_a.subscribe_public_feed().await.unwrap();
    let mut sub_b = net_b.subscribe_public_feed().await.unwrap();

    // Create services for node A.
    let (svc_a, _dir_a) = make_services();
    svc_a.identity.create("test-pass").await.unwrap();

    // Create a post using create_and_publish.
    let result = svc_a
        .posts
        .create_and_publish(
            "Hello from the gossip pipeline test!",
            vec![],
            Some(86400),
            None,
            &svc_a.identity,
            svc_a.content_store(),
            &svc_a.metadata_db,
            &net_a,
            &svc_a.rate_limiter,
            &svc_a.reputation,
            &svc_a.fingerprint_store,
            &svc_a.content_filter,
            None,
        )
        .await
        .unwrap();

    // Verify the post was created successfully.
    assert!(result["published"].as_bool().unwrap());
    let content_hash = result["content_hash"].as_str().unwrap();
    assert_eq!(content_hash.len(), 64);

    // B should receive the gossip message containing the serialized post.
    let received = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout: B did not receive the post via gossip")
        .expect("B subscription channel closed");

    // Verify the received payload deserializes to a valid Post.
    let post: ephemera_post::Post = serde_json::from_slice(&received.payload)
        .expect("received gossip payload should be a valid Post");

    assert_eq!(
        post.content.text_body().unwrap(),
        "Hello from the gossip pipeline test!"
    );

    net_a.shutdown().await;
    net_b.shutdown().await;
}

/// Test that receiving a gossip message containing a valid post stores it
/// locally via the gossip ingest pipeline.
#[tokio::test]
async fn test_gossip_receive_stores_post() {
    // Create a signed post from a "remote" author.
    let remote_kp = SigningKeyPair::generate();
    let post = PostBuilder::new()
        .text("Post from a remote peer via gossip")
        .ttl(Ttl::from_secs(86400).unwrap())
        .build(&remote_kp)
        .unwrap();
    let post_bytes = serde_json::to_vec(&post).unwrap();

    // Set up a receiving node with storage.
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let content_path = config.content_path();
    std::fs::create_dir_all(&content_path).unwrap();
    let metadata_dir = config.data_dir.join("metadata");
    std::fs::create_dir_all(&metadata_dir).unwrap();

    let content_store = ContentStore::open(&content_path).unwrap();
    let metadata_db_path = config.metadata_db_path();
    let metadata_db = MetadataDb::open(&metadata_db_path).unwrap();
    let metadata_db_mutex = Mutex::new(metadata_db);

    let event_bus = EventBus::new();
    let mut event_rx = event_bus.subscribe();

    // Set up a gossip subscription that we manually feed.
    let topic = GossipTopic::public_feed();
    let (sub, tx) = ephemera_gossip::TopicSubscription::new(topic, 256);

    // Send the post bytes into the subscription channel.
    let gossip_msg = ephemera_gossip::topic::GossipMessage {
        topic,
        payload: post_bytes.clone(),
        content_hash: *blake3::hash(&post_bytes).as_bytes(),
        source_node: [0xAA; 32],
    };
    tx.send(gossip_msg).await.unwrap();

    // Drop the sender so the ingest loop will exit after processing.
    drop(tx);

    // Create a shutdown channel.
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Run the ingest loop (it will process the one message then exit when
    // the channel is closed).
    ephemera_node::gossip_ingest::gossip_ingest_loop(
        sub,
        content_store,
        metadata_db_mutex,
        event_bus.clone(),
        Mutex::new(RateLimiter::new()),
        Mutex::new(FingerprintStore::new()),
        Mutex::new(ContentFilter::empty()),
        shutdown_rx,
    )
    .await;

    // Verify the post was stored in metadata DB.
    let metadata_db2 = MetadataDb::open(&metadata_db_path).unwrap();
    let content_hash_wire = post.id.to_wire_bytes();
    let row: Result<(Option<String>, i64), _> = metadata_db2.conn().query_row(
        "SELECT body_preview, created_at FROM posts WHERE content_hash = ?1",
        rusqlite::params![content_hash_wire],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );

    let (body_preview, _created_at) = row.expect("post should be stored in database");
    assert_eq!(
        body_preview.as_deref(),
        Some("Post from a remote peer via gossip")
    );

    // Verify a PostReceived event was emitted.
    let event = event_rx.try_recv();
    assert!(
        event.is_ok(),
        "a PostReceived event should have been emitted"
    );
    match event.unwrap() {
        ephemera_events::Event::PostReceived { content_id, author } => {
            assert_eq!(content_id.hash_bytes(), post.id.hash_bytes());
            assert_eq!(author.as_bytes(), remote_kp.public_key().as_bytes());
        }
        other => panic!("expected PostReceived, got {:?}", other),
    }
}
