use super::feed_discover::insert_test_post;
use super::*;

fn alice() -> IdentityKey {
    IdentityKey::from_bytes([0x01; 32])
}

fn bob() -> IdentityKey {
    IdentityKey::from_bytes([0x02; 32])
}

fn carol() -> IdentityKey {
    IdentityKey::from_bytes([0x03; 32])
}

fn dave() -> IdentityKey {
    IdentityKey::from_bytes([0x04; 32])
}

fn make_services() -> SqliteSocialServices {
    let db = MetadataDb::open_in_memory().unwrap();
    SqliteSocialServices::new(db)
}

// ── Connection tests ────────────────────────────────────────────

#[tokio::test]
async fn test_connection_request_roundtrip() {
    let services = make_services();

    // Alice sends a connection request to Bob.
    let conn = services
        .request(&alice(), &bob(), Some("Hey Bob!"))
        .await
        .unwrap();
    assert_eq!(conn.status, ConnectionStatus::PendingOutgoing);
    assert_eq!(conn.initiator, alice());
    assert_eq!(conn.responder, bob());
    assert_eq!(conn.message.as_deref(), Some("Hey Bob!"));

    // Simulate Bob receiving the request (store as PendingIncoming on Bob's side).
    {
        let db = services.db.lock().unwrap();
        let result = receive_connection_request(&db, &bob(), &alice(), Some("Hey Bob!"));
        assert!(result.is_ok());
        let incoming = result.unwrap();
        assert!(incoming.is_some());
        let incoming = incoming.unwrap();
        assert_eq!(incoming.status, ConnectionStatus::PendingIncoming);
    }

    // Bob accepts.
    let accepted = services.accept(&bob(), &alice()).await.unwrap();
    assert_eq!(accepted.status, ConnectionStatus::Active);

    // Verify both sides are connected.
    let alice_conns = services
        .list(&alice(), Some(ConnectionStatus::Active))
        .await
        .unwrap();
    assert_eq!(alice_conns.len(), 1);
    assert_eq!(alice_conns[0].status, ConnectionStatus::Active);

    let bob_conns = services
        .list(&bob(), Some(ConnectionStatus::Active))
        .await
        .unwrap();
    assert_eq!(bob_conns.len(), 1);
    assert_eq!(bob_conns[0].status, ConnectionStatus::Active);
}

#[tokio::test]
async fn test_connection_reject() {
    let services = make_services();

    // Alice sends a request.
    services.request(&alice(), &bob(), None).await.unwrap();

    // Bob receives it.
    {
        let db = services.db.lock().unwrap();
        receive_connection_request(&db, &bob(), &alice(), None).unwrap();
    }

    // Bob rejects.
    services.reject(&bob(), &alice()).await.unwrap();

    // Verify Bob has no connections.
    let bob_conns = services.list(&bob(), None).await.unwrap();
    assert!(bob_conns.is_empty());

    // Alice still has PendingOutgoing (she hasn't been notified yet).
    let alice_conns = services
        .list(&alice(), Some(ConnectionStatus::PendingOutgoing))
        .await
        .unwrap();
    assert_eq!(alice_conns.len(), 1);
}

#[tokio::test]
async fn test_duplicate_request_rejected() {
    let services = make_services();

    // First request succeeds.
    services.request(&alice(), &bob(), None).await.unwrap();

    // Second request should fail with AlreadyExists.
    let result = services.request(&alice(), &bob(), None).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        ConnectionError::AlreadyExists { status } => {
            assert_eq!(status, ConnectionStatus::PendingOutgoing);
        }
        other => panic!("expected AlreadyExists, got: {other:?}"),
    }
}

// ── Feed tests ──────────────────────────────────────────────────

#[tokio::test]
async fn test_feed_filters_to_connections() {
    let services = make_services();

    // Insert posts from alice, bob, and carol.
    let now = Timestamp::now().as_secs() as i64;
    {
        let db = services.db.lock().unwrap();
        insert_test_post(&db, &alice(), now - 300, 86400);
        insert_test_post(&db, &bob(), now - 200, 86400);
        insert_test_post(&db, &carol(), now - 100, 86400);
    }

    // Alice connects to Bob (full flow).
    services.request(&alice(), &bob(), None).await.unwrap();
    {
        let db = services.db.lock().unwrap();
        receive_connection_request(&db, &bob(), &alice(), None).unwrap();
    }
    services.accept(&bob(), &alice()).await.unwrap();

    // Alice's connections feed should show alice's own post + bob's post.
    let feed = services.connections_feed(&alice(), None, 50).await.unwrap();

    let authors: Vec<IdentityKey> = feed.items.iter().map(|i| i.author).collect();
    assert!(
        authors.contains(&alice()),
        "feed should contain alice's posts"
    );
    assert!(authors.contains(&bob()), "feed should contain bob's posts");
    assert!(
        !authors.contains(&carol()),
        "feed should NOT contain carol's posts (not connected)"
    );
}

#[tokio::test]
async fn test_block_hides_posts() {
    let services = make_services();

    let now = Timestamp::now().as_secs() as i64;

    // Alice connects to Bob.
    services.request(&alice(), &bob(), None).await.unwrap();
    {
        let db = services.db.lock().unwrap();
        receive_connection_request(&db, &bob(), &alice(), None).unwrap();
    }
    services.accept(&bob(), &alice()).await.unwrap();

    // Insert a post from Bob.
    {
        let db = services.db.lock().unwrap();
        insert_test_post(&db, &bob(), now - 100, 86400);
    }

    // Alice can see Bob's post.
    let feed_before = services.connections_feed(&alice(), None, 50).await.unwrap();
    assert!(
        feed_before.items.iter().any(|i| i.author == bob()),
        "alice should see bob's post before blocking"
    );

    // Alice blocks Bob.
    services
        .block(&alice(), &bob(), Some("spam"))
        .await
        .unwrap();

    // Alice can no longer see Bob's post.
    let feed_after = services.connections_feed(&alice(), None, 50).await.unwrap();
    assert!(
        !feed_after.items.iter().any(|i| i.author == bob()),
        "alice should NOT see bob's post after blocking"
    );
}

#[tokio::test]
async fn test_blocked_user_request_auto_rejected() {
    let services = make_services();

    // Alice blocks Carol.
    services.block(&alice(), &carol(), None).await.unwrap();

    // Carol tries to send a request to Alice.
    {
        let db = services.db.lock().unwrap();
        let result = receive_connection_request(&db, &alice(), &carol(), Some("hi"));
        assert!(result.is_ok());
        // Should return None (silently discarded).
        assert!(result.unwrap().is_none());
    }

    // Alice should have no pending incoming connections.
    let incoming = services
        .list(&alice(), Some(ConnectionStatus::PendingIncoming))
        .await
        .unwrap();
    assert!(incoming.is_empty());
}

#[tokio::test]
async fn test_muted_user_posts_hidden() {
    let services = make_services();

    let now = Timestamp::now().as_secs() as i64;

    // Alice connects to Dave.
    services.request(&alice(), &dave(), None).await.unwrap();
    {
        let db = services.db.lock().unwrap();
        receive_connection_request(&db, &dave(), &alice(), None).unwrap();
    }
    services.accept(&dave(), &alice()).await.unwrap();

    // Insert a post from Dave.
    {
        let db = services.db.lock().unwrap();
        insert_test_post(&db, &dave(), now - 50, 86400);
    }

    // Alice mutes Dave (permanent).
    services.mute(&alice(), &dave(), None).await.unwrap();

    // Alice's feed should not show Dave's post.
    let feed = services.connections_feed(&alice(), None, 50).await.unwrap();
    assert!(
        !feed.items.iter().any(|i| i.author == dave()),
        "muted user's posts should be hidden"
    );
}

// Remaining tests in the _extra module.
#[path = "store_tests_extra.rs"]
mod extra;
