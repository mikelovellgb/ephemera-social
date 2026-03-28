use super::*;

fn insert_post(db: &MetadataDb, author: &[u8], created_at: i64) {
    let mut ch = vec![0x01u8];
    ch.extend_from_slice(blake3::hash(&created_at.to_le_bytes()).as_bytes());
    db.conn()
        .execute(
            "INSERT INTO posts (content_hash, author_pubkey, sequence_number,
             created_at, expires_at, ttl_seconds, received_at, epoch_number,
             signature) VALUES (?1,?2,?3,?4,?5,86400,?4,1,?6)",
            rusqlite::params![
                ch,
                author,
                created_at,
                created_at,
                created_at + 86400,
                vec![0xBBu8; 64]
            ],
        )
        .unwrap();
}

#[test]
fn feed_query_empty() {
    let db = MetadataDb::open_in_memory().unwrap();
    let results = QueryEngine::feed_query(&db, &[vec![0xAAu8; 32]], None, 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn feed_query_returns_posts_ordered() {
    let db = MetadataDb::open_in_memory().unwrap();
    let author = vec![0xAAu8; 32];

    insert_post(&db, &author, 1_000);
    insert_post(&db, &author, 2_000);
    insert_post(&db, &author, 3_000);

    let results = QueryEngine::feed_query(&db, &[author], None, 10).unwrap();
    assert_eq!(results.len(), 3);
    // Should be ordered by created_at DESC.
    assert!(results[0].created_at >= results[1].created_at);
    assert!(results[1].created_at >= results[2].created_at);
}

#[test]
fn feed_query_cursor_pagination() {
    let db = MetadataDb::open_in_memory().unwrap();
    let author = vec![0xAAu8; 32];

    for i in 1..=5 {
        insert_post(&db, &author, i * 1000);
    }

    let page1 = QueryEngine::feed_query(&db, std::slice::from_ref(&author), None, 2).unwrap();
    assert_eq!(page1.len(), 2);

    let cursor = page1.last().unwrap().created_at;
    let page2 = QueryEngine::feed_query(&db, &[author], Some(cursor), 2).unwrap();
    assert_eq!(page2.len(), 2);

    // No overlap.
    assert!(page2[0].created_at < page1[1].created_at);
}

#[test]
fn connections_query_with_filter() {
    let db = MetadataDb::open_in_memory().unwrap();
    let (local, r1, r2) = (vec![0xAAu8; 32], vec![0xBBu8; 32], vec![0xCCu8; 32]);
    let sql = "INSERT INTO connections (local_pubkey, remote_pubkey, status, created_at, updated_at) VALUES (?1,?2,?3,?4,?4)";
    db.conn()
        .execute(sql, rusqlite::params![local, r1, "connected", 100])
        .unwrap();
    db.conn()
        .execute(sql, rusqlite::params![local, r2, "pending_outgoing", 200])
        .unwrap();

    assert_eq!(
        QueryEngine::connections_query(&db, &local, None)
            .unwrap()
            .len(),
        2
    );
    let connected = QueryEngine::connections_query(&db, &local, Some("connected")).unwrap();
    assert_eq!(connected.len(), 1);
    assert_eq!(connected[0].status, "connected");
}
