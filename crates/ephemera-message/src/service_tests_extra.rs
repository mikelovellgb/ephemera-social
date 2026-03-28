use super::*;
use ephemera_types::{Timestamp, Ttl};

#[test]
fn test_conversation_list_ordering() {
    let db = setup_test_db();
    let (alice_id, _alice_x25519) = alice_keys();
    let (bob_id, _bob_x25519) = bob_keys();
    let (carol_id, _carol_x25519) = carol_keys();
    let (dave_id, _dave_x25519) = dave_keys();

    let alice_bytes = alice_id.as_bytes().to_vec();
    let bob_bytes = bob_id.as_bytes().to_vec();
    let carol_bytes = carol_id.as_bytes().to_vec();
    let dave_bytes = dave_id.as_bytes().to_vec();

    make_connected(&db, &alice_bytes, &bob_bytes);
    make_connected(&db, &alice_bytes, &carol_bytes);
    make_connected(&db, &alice_bytes, &dave_bytes);

    let conv_bob = MessageService::compute_conversation_id(&alice_bytes, &bob_bytes);
    let conv_carol = MessageService::compute_conversation_id(&alice_bytes, &carol_bytes);
    let conv_dave = MessageService::compute_conversation_id(&alice_bytes, &dave_bytes);

    let base_time = Timestamp::now().as_secs() as i64;

    db.conn()
        .execute(
            "INSERT INTO conversations (conversation_id, our_pubkey, their_pubkey,
             last_message_at, unread_count, is_request, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?4)",
            rusqlite::params![conv_bob, alice_bytes, bob_bytes, base_time - 300],
        )
        .unwrap();

    db.conn()
        .execute(
            "INSERT INTO conversations (conversation_id, our_pubkey, their_pubkey,
             last_message_at, unread_count, is_request, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?4)",
            rusqlite::params![conv_carol, alice_bytes, carol_bytes, base_time - 200],
        )
        .unwrap();

    db.conn()
        .execute(
            "INSERT INTO conversations (conversation_id, our_pubkey, their_pubkey,
             last_message_at, unread_count, is_request, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?4)",
            rusqlite::params![conv_dave, alice_bytes, dave_bytes, base_time - 100],
        )
        .unwrap();

    let conversations = MessageService::list_conversations(&db, &alice_bytes).unwrap();
    assert_eq!(conversations.len(), 3);

    assert_eq!(conversations[0].their_pubkey, dave_bytes);
    assert_eq!(conversations[1].their_pubkey, carol_bytes);
    assert_eq!(conversations[2].their_pubkey, bob_bytes);
}

#[test]
fn test_message_ttl_gc() {
    let db = setup_test_db();
    let (alice_id, _) = alice_keys();
    let (bob_id, _) = bob_keys();

    let alice_bytes = alice_id.as_bytes().to_vec();
    let bob_bytes = bob_id.as_bytes().to_vec();

    let conv_id = MessageService::compute_conversation_id(&alice_bytes, &bob_bytes);

    let now = Timestamp::now().as_secs() as i64;
    db.conn()
        .execute(
            "INSERT INTO conversations (conversation_id, our_pubkey, their_pubkey,
             last_message_at, unread_count, is_request, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?4)",
            rusqlite::params![conv_id, alice_bytes, bob_bytes, now],
        )
        .unwrap();

    let expired_at = now - 1000;
    let msg_id = blake3::hash(b"expired-msg").as_bytes().to_vec();
    db.conn()
        .execute(
            "INSERT INTO messages (message_id, conversation_id, sender_pubkey,
             received_at, expires_at, is_read, body_preview, has_media)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 'old message', 0)",
            rusqlite::params![msg_id, conv_id, alice_bytes, now - 2000, expired_at],
        )
        .unwrap();

    let future_expires = now + 86400;
    let msg_id2 = blake3::hash(b"fresh-msg").as_bytes().to_vec();
    db.conn()
        .execute(
            "INSERT INTO messages (message_id, conversation_id, sender_pubkey,
             received_at, expires_at, is_read, body_preview, has_media)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 'fresh message', 0)",
            rusqlite::params![msg_id2, conv_id, alice_bytes, now, future_expires],
        )
        .unwrap();

    let count_before: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
            rusqlite::params![conv_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count_before, 2);

    let deleted = MessageService::gc_expired_messages(&db).unwrap();
    assert_eq!(deleted, 1, "should delete exactly the expired message");

    let count_after: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
            rusqlite::params![conv_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count_after, 1);

    let remaining_preview: String = db
        .conn()
        .query_row(
            "SELECT body_preview FROM messages WHERE conversation_id = ?1",
            rusqlite::params![conv_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_preview, "fresh message");
}

#[test]
fn test_send_and_receive_connected_users() {
    let db = setup_test_db();
    let (alice_id, _alice_x25519) = alice_keys();
    let (bob_id, bob_x25519) = bob_keys();

    let alice_bytes = alice_id.as_bytes().to_vec();
    let bob_bytes = bob_id.as_bytes().to_vec();

    make_connected(&db, &alice_bytes, &bob_bytes);
    make_connected(&db, &bob_bytes, &alice_bytes);

    let plaintext = b"Hey Bob, how's it going?";
    let (_conv_id, envelope) = MessageService::send_message(
        &db,
        &alice_id,
        &bob_id,
        &bob_x25519.public,
        plaintext,
        Ttl::one_day(),
    )
    .unwrap();

    let payload =
        MessageService::receive_message(&db, &envelope, &bob_id, &bob_x25519.secret).unwrap();

    assert_eq!(payload.sender, alice_id);
    assert_eq!(payload.body, plaintext);

    let convs = MessageService::list_conversations(&db, &alice_bytes).unwrap();
    assert!(!convs.is_empty());
}

#[test]
fn test_get_messages_pagination() {
    let db = setup_test_db();
    let (alice_id, _) = alice_keys();
    let (bob_id, _) = bob_keys();

    let alice_bytes = alice_id.as_bytes().to_vec();
    let bob_bytes = bob_id.as_bytes().to_vec();

    let conv_id = MessageService::compute_conversation_id(&alice_bytes, &bob_bytes);
    let now = Timestamp::now().as_secs() as i64;

    db.conn()
        .execute(
            "INSERT INTO conversations (conversation_id, our_pubkey, their_pubkey,
             last_message_at, unread_count, is_request, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?4)",
            rusqlite::params![conv_id, alice_bytes, bob_bytes, now],
        )
        .unwrap();

    for i in 0..5 {
        let msg_id = blake3::hash(format!("msg-{i}").as_bytes())
            .as_bytes()
            .to_vec();
        db.conn()
            .execute(
                "INSERT INTO messages (message_id, conversation_id, sender_pubkey,
                 received_at, expires_at, is_read, body_preview, has_media)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, 0)",
                rusqlite::params![
                    msg_id,
                    conv_id,
                    alice_bytes,
                    now + i,
                    now + 86400,
                    format!("message {i}")
                ],
            )
            .unwrap();
    }

    let page1 = MessageService::get_messages(&db, &conv_id, None, 2).unwrap();
    assert_eq!(page1.len(), 2);

    let cursor = page1.last().unwrap().received_at;
    let page2 = MessageService::get_messages(&db, &conv_id, Some(cursor), 2).unwrap();
    assert_eq!(page2.len(), 2);

    assert!(page2[0].received_at < page1.last().unwrap().received_at);
}
