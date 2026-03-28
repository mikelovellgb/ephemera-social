use super::*;

#[tokio::test]
async fn test_discover_feed_excludes_connections() {
    let services = make_services();

    let now = Timestamp::now().as_secs() as i64;

    // Alice connects to Bob.
    services.request(&alice(), &bob(), None).await.unwrap();
    {
        let db = services.db.lock().unwrap();
        receive_connection_request(&db, &bob(), &alice(), None).unwrap();
    }
    services.accept(&bob(), &alice()).await.unwrap();

    // Insert posts from Bob (connected) and Carol (not connected).
    {
        let db = services.db.lock().unwrap();
        insert_test_post(&db, &bob(), now - 200, 86400);
        insert_test_post(&db, &carol(), now - 100, 86400);
    }

    // Discover feed should show Carol but not Bob or Alice.
    let discover = discover_feed(&services, &alice(), None, 50).await.unwrap();
    let authors: Vec<IdentityKey> = discover.items.iter().map(|i| i.author).collect();
    assert!(
        !authors.contains(&bob()),
        "discover should not contain connected user"
    );
    assert!(
        !authors.contains(&alice()),
        "discover should not contain self"
    );
    assert!(
        authors.contains(&carol()),
        "discover should contain non-connected user"
    );
}

#[tokio::test]
async fn test_connection_remove() {
    let services = make_services();

    // Full connection flow.
    services.request(&alice(), &bob(), None).await.unwrap();
    {
        let db = services.db.lock().unwrap();
        receive_connection_request(&db, &bob(), &alice(), None).unwrap();
    }
    services.accept(&bob(), &alice()).await.unwrap();

    // Remove the connection.
    services.remove(&alice(), &bob()).await.unwrap();

    // Both sides should have no connections.
    let alice_conns = services.list(&alice(), None).await.unwrap();
    assert!(alice_conns.is_empty());

    let bob_conns = services.list(&bob(), None).await.unwrap();
    assert!(bob_conns.is_empty());
}

#[tokio::test]
async fn test_message_too_long() {
    let services = make_services();
    let long_msg = "x".repeat(281);
    let result = services.request(&alice(), &bob(), Some(&long_msg)).await;
    assert!(matches!(
        result,
        Err(ConnectionError::MessageTooLong { len: 281 })
    ));
}

// ---- Reaction tests ----

use crate::interaction::ReactionEmoji;

#[test]
fn test_react_and_get_counts() {
    let services = make_services();
    let post = "abc123";
    let reactor = hex::encode(alice().as_bytes());

    services
        .react(post, &reactor, ReactionEmoji::Heart, 1000)
        .unwrap();

    let summary = services.get_reactions(post, None).unwrap();
    assert_eq!(summary.count_for(ReactionEmoji::Heart), 1);
    assert_eq!(summary.total(), 1);
}

#[test]
fn test_unreact() {
    let services = make_services();
    let post = "post_unreact";
    let reactor = hex::encode(alice().as_bytes());

    services
        .react(post, &reactor, ReactionEmoji::Fire, 1000)
        .unwrap();
    assert_eq!(
        services
            .get_reactions(post, None)
            .unwrap()
            .count_for(ReactionEmoji::Fire),
        1
    );

    services.unreact(post, &reactor).unwrap();
    let summary = services.get_reactions(post, None).unwrap();
    assert_eq!(summary.count_for(ReactionEmoji::Fire), 0);
    assert_eq!(summary.total(), 0);
}

#[test]
fn test_multiple_reactions() {
    let services = make_services();
    let post = "post_multi";

    let alice_pk = hex::encode(alice().as_bytes());
    let bob_pk = hex::encode(bob().as_bytes());
    let carol_pk = hex::encode(carol().as_bytes());

    services
        .react(post, &alice_pk, ReactionEmoji::Heart, 1000)
        .unwrap();
    services
        .react(post, &bob_pk, ReactionEmoji::Heart, 1001)
        .unwrap();
    services
        .react(post, &carol_pk, ReactionEmoji::Fire, 1002)
        .unwrap();

    let summary = services.get_reactions(post, None).unwrap();
    assert_eq!(summary.count_for(ReactionEmoji::Heart), 2);
    assert_eq!(summary.count_for(ReactionEmoji::Fire), 1);
    assert_eq!(summary.total(), 3);
}

#[test]
fn test_one_reaction_per_user() {
    let services = make_services();
    let post = "post_one_per_user";
    let reactor = hex::encode(alice().as_bytes());

    services
        .react(post, &reactor, ReactionEmoji::Heart, 1000)
        .unwrap();
    services
        .react(post, &reactor, ReactionEmoji::Fire, 1001)
        .unwrap();

    let summary = services.get_reactions(post, Some(&reactor)).unwrap();
    assert_eq!(summary.count_for(ReactionEmoji::Heart), 0, "old reaction gone");
    assert_eq!(summary.count_for(ReactionEmoji::Fire), 1, "new reaction present");
    assert_eq!(summary.total(), 1, "only one reaction total");
    assert_eq!(summary.my_emoji, Some(ReactionEmoji::Fire));
}

// ---- Topic room tests ----

#[test]
fn test_create_topic_room() {
    let services = make_services();
    let creator = hex::encode(alice().as_bytes());

    let room = services
        .create_topic("rust-lang", Some("Talk about Rust"), &creator, 1000)
        .unwrap();
    assert_eq!(room.name, "rust-lang");
    assert_eq!(room.description.as_deref(), Some("Talk about Rust"));
    assert_eq!(room.created_by, creator);

    let topics = services.list_topics().unwrap();
    assert_eq!(topics.len(), 1);
    assert_eq!(topics[0].name, "rust-lang");
}

#[test]
fn test_topic_subscription() {
    let services = make_services();
    let creator = hex::encode(alice().as_bytes());
    let joiner = hex::encode(bob().as_bytes());

    let room = services
        .create_topic("go-lang", None, &creator, 1000)
        .unwrap();

    services.join_topic(&room.topic_id, &joiner, 1001).unwrap();
    services.leave_topic(&room.topic_id, &joiner).unwrap();

    let topics = services.list_topics().unwrap();
    assert_eq!(topics.len(), 1);
}

#[test]
fn test_topic_feed() {
    let services = make_services();
    let creator = hex::encode(alice().as_bytes());

    let room = services
        .create_topic("test-feed", None, &creator, 1000)
        .unwrap();

    let now = Timestamp::now().as_secs() as i64;
    {
        let db = services.db.lock().unwrap();
        insert_test_post(&db, &alice(), now - 100, 86400);
    }

    let content_hash = {
        let db = services.db.lock().unwrap();
        let hash: Vec<u8> = db
            .conn()
            .query_row(
                "SELECT content_hash FROM posts WHERE author_pubkey = ?1",
                rusqlite::params![alice().as_bytes().to_vec()],
                |row| row.get(0),
            )
            .unwrap();
        hash
    };

    services
        .post_to_topic(&room.topic_id, &content_hash, now - 100)
        .unwrap();

    let feed = services.get_topic_feed(&room.topic_id, None, 50).unwrap();
    assert_eq!(feed.items.len(), 1, "topic feed should contain the posted item");
    assert_eq!(feed.items[0].author, alice());
}

#[test]
fn test_join_nonexistent_topic_fails() {
    let services = make_services();
    let joiner = hex::encode(alice().as_bytes());
    let result = services.join_topic("nonexistent_topic_id", &joiner, 1000);
    assert!(result.is_err());
}

#[test]
fn test_duplicate_topic_creation_fails() {
    let services = make_services();
    let creator = hex::encode(alice().as_bytes());

    services
        .create_topic("same-name", None, &creator, 1000)
        .unwrap();

    let result = services.create_topic("same-name", None, &creator, 1001);
    assert!(result.is_err());
}
