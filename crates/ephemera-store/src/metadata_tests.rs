use super::*;

#[test]
fn open_in_memory() {
    let db = MetadataDb::open_in_memory().unwrap();
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn schema_creates_all_tables() {
    let db = MetadataDb::open_in_memory().unwrap();
    let tables: Vec<String> = db
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let expected = [
        "blocks",
        "connections",
        "conversations",
        "epoch_keys",
        "follows",
        "local_state",
        "message_requests",
        "messages",
        "mutes",
        "post_mentions",
        "post_tags",
        "posts",
        "profiles",
        "schema_version",
    ];
    for table in &expected {
        assert!(
            tables.contains(&(*table).to_string()),
            "missing table: {table}"
        );
    }
}

#[test]
fn insert_and_query_post() {
    let db = MetadataDb::open_in_memory().unwrap();
    let now = 1_700_000_000i64;
    let hash = vec![0x01u8; 33];
    let author = vec![0xAAu8; 32];
    let sig = vec![0xBBu8; 64];

    db.conn()
        .execute(
            "INSERT INTO posts (
                    content_hash, author_pubkey, sequence_number, created_at,
                    expires_at, ttl_seconds, received_at, epoch_number, signature
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![hash, author, 1, now, now + 86400, 86400, now, 1, sig],
        )
        .unwrap();

    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM posts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
