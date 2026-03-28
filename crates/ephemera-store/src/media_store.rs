//! Media attachment storage operations.
//!
//! Provides CRUD operations for media attachments and their chunks
//! against the SQLite metadata database. Media blobs are chunked for
//! distributed storage; each chunk is independently hashed for integrity.

use crate::MetadataDb;
use crate::StoreError;

/// Metadata about a stored media attachment.
#[derive(Debug, Clone)]
pub struct MediaAttachment {
    /// Unique identifier for this attachment.
    pub id: String,
    /// Wire bytes of the parent post's content hash.
    pub post_hash: Vec<u8>,
    /// BLAKE3 hex hash of the processed media content.
    pub content_hash: String,
    /// "image" or "video".
    pub media_type: String,
    /// MIME type (e.g. "image/png", "video/mp4").
    pub mime_type: String,
    /// File size in bytes.
    pub file_size: i64,
    /// Width in pixels (if known).
    pub width: Option<i64>,
    /// Height in pixels (if known).
    pub height: Option<i64>,
    /// Duration in milliseconds (video only).
    pub duration_ms: Option<i64>,
    /// BLAKE3 hex hash of the thumbnail blob.
    pub thumbnail_hash: Option<String>,
    /// Number of chunks the content was split into.
    pub chunk_count: i64,
    /// Creation timestamp (Unix seconds).
    pub created_at: i64,
}

/// A single stored chunk of media data.
#[derive(Debug, Clone)]
pub struct StoredChunk {
    /// BLAKE3 hex hash of this chunk's data.
    pub chunk_hash: String,
    /// ID of the parent media attachment.
    pub media_id: String,
    /// Zero-based index within the media.
    pub chunk_index: i64,
    /// The raw chunk bytes.
    pub data: Vec<u8>,
}

/// Insert a media attachment record into the database.
pub fn insert_media_attachment(
    db: &MetadataDb,
    attachment: &MediaAttachment,
) -> Result<(), StoreError> {
    db.conn().execute(
        "INSERT INTO media_attachments (
            id, post_hash, content_hash, media_type, mime_type,
            file_size, width, height, duration_ms, thumbnail_hash,
            chunk_count, created_at
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        rusqlite::params![
            attachment.id,
            attachment.post_hash,
            attachment.content_hash,
            attachment.media_type,
            attachment.mime_type,
            attachment.file_size,
            attachment.width,
            attachment.height,
            attachment.duration_ms,
            attachment.thumbnail_hash,
            attachment.chunk_count,
            attachment.created_at,
        ],
    )?;
    Ok(())
}

/// Insert a single media chunk into the database.
pub fn insert_media_chunk(db: &MetadataDb, chunk: &StoredChunk) -> Result<(), StoreError> {
    db.conn().execute(
        "INSERT INTO media_chunks (chunk_hash, media_id, chunk_index, data)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![chunk.chunk_hash, chunk.media_id, chunk.chunk_index, chunk.data,],
    )?;
    Ok(())
}

/// Retrieve media attachment metadata by its ID.
pub fn get_media_attachment(
    db: &MetadataDb,
    media_id: &str,
) -> Result<MediaAttachment, StoreError> {
    db.conn()
        .query_row(
            "SELECT id, post_hash, content_hash, media_type, mime_type,
                    file_size, width, height, duration_ms, thumbnail_hash,
                    chunk_count, created_at
             FROM media_attachments WHERE id = ?1",
            rusqlite::params![media_id],
            |row| {
                Ok(MediaAttachment {
                    id: row.get(0)?,
                    post_hash: row.get(1)?,
                    content_hash: row.get(2)?,
                    media_type: row.get(3)?,
                    mime_type: row.get(4)?,
                    file_size: row.get(5)?,
                    width: row.get(6)?,
                    height: row.get(7)?,
                    duration_ms: row.get(8)?,
                    thumbnail_hash: row.get(9)?,
                    chunk_count: row.get(10)?,
                    created_at: row.get(11)?,
                })
            },
        )
        .map_err(|e| StoreError::NotFound(format!("media attachment {media_id}: {e}")))
}

/// Retrieve a single chunk by its hash.
pub fn get_media_chunk(db: &MetadataDb, chunk_hash: &str) -> Result<StoredChunk, StoreError> {
    db.conn()
        .query_row(
            "SELECT chunk_hash, media_id, chunk_index, data
             FROM media_chunks WHERE chunk_hash = ?1",
            rusqlite::params![chunk_hash],
            |row| {
                Ok(StoredChunk {
                    chunk_hash: row.get(0)?,
                    media_id: row.get(1)?,
                    chunk_index: row.get(2)?,
                    data: row.get(3)?,
                })
            },
        )
        .map_err(|e| StoreError::NotFound(format!("media chunk {chunk_hash}: {e}")))
}

/// List all media attachments for a given post (by its wire-encoded hash).
pub fn list_media_for_post(
    db: &MetadataDb,
    post_hash_wire: &[u8],
) -> Result<Vec<MediaAttachment>, StoreError> {
    let mut stmt = db.conn().prepare(
        "SELECT id, post_hash, content_hash, media_type, mime_type,
                file_size, width, height, duration_ms, thumbnail_hash,
                chunk_count, created_at
         FROM media_attachments WHERE post_hash = ?1
         ORDER BY created_at ASC",
    )?;

    let rows = stmt
        .query_map(rusqlite::params![post_hash_wire], |row| {
            Ok(MediaAttachment {
                id: row.get(0)?,
                post_hash: row.get(1)?,
                content_hash: row.get(2)?,
                media_type: row.get(3)?,
                mime_type: row.get(4)?,
                file_size: row.get(5)?,
                width: row.get(6)?,
                height: row.get(7)?,
                duration_ms: row.get(8)?,
                thumbnail_hash: row.get(9)?,
                chunk_count: row.get(10)?,
                created_at: row.get(11)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// List all chunks for a media attachment, ordered by index.
pub fn list_chunks_for_media(
    db: &MetadataDb,
    media_id: &str,
) -> Result<Vec<StoredChunk>, StoreError> {
    let mut stmt = db.conn().prepare(
        "SELECT chunk_hash, media_id, chunk_index, data
         FROM media_chunks WHERE media_id = ?1
         ORDER BY chunk_index ASC",
    )?;

    let rows = stmt
        .query_map(rusqlite::params![media_id], |row| {
            Ok(StoredChunk {
                chunk_hash: row.get(0)?,
                media_id: row.get(1)?,
                chunk_index: row.get(2)?,
                data: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Retrieve the thumbnail blob hash for a media attachment.
pub fn get_thumbnail_hash(
    db: &MetadataDb,
    media_id: &str,
) -> Result<Option<String>, StoreError> {
    let hash: Option<String> = db
        .conn()
        .query_row(
            "SELECT thumbnail_hash FROM media_attachments WHERE id = ?1",
            rusqlite::params![media_id],
            |row| row.get(0),
        )
        .map_err(|e| StoreError::NotFound(format!("media {media_id}: {e}")))?;
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> MetadataDb {
        MetadataDb::open_in_memory().unwrap()
    }

    fn dummy_post_hash() -> Vec<u8> {
        // Insert a dummy post so FK constraint is satisfied.
        vec![0xAA; 32]
    }

    fn insert_dummy_post(db: &MetadataDb) -> Vec<u8> {
        let hash = dummy_post_hash();
        let sig = vec![0u8; 64];
        db.conn()
            .execute(
                "INSERT INTO posts (
                    content_hash, author_pubkey, sequence_number, created_at,
                    expires_at, ttl_seconds, depth, media_count, has_media,
                    pow_difficulty, received_at, epoch_number, signature
                ) VALUES (?1,?2,0,1000,2000,1000,0,0,0,0,1000,1,?3)",
                rusqlite::params![hash, vec![0xBBu8; 32], sig],
            )
            .unwrap();
        hash
    }

    fn sample_attachment(post_hash: Vec<u8>) -> MediaAttachment {
        MediaAttachment {
            id: "media-001".to_string(),
            post_hash,
            content_hash: "abc123".to_string(),
            media_type: "image".to_string(),
            mime_type: "image/png".to_string(),
            file_size: 1024,
            width: Some(640),
            height: Some(480),
            duration_ms: None,
            thumbnail_hash: Some("thumb-hash".to_string()),
            chunk_count: 2,
            created_at: 1000,
        }
    }

    #[test]
    fn insert_and_get_attachment() {
        let db = test_db();
        let post_hash = insert_dummy_post(&db);
        let att = sample_attachment(post_hash);
        insert_media_attachment(&db, &att).unwrap();

        let loaded = get_media_attachment(&db, "media-001").unwrap();
        assert_eq!(loaded.content_hash, "abc123");
        assert_eq!(loaded.media_type, "image");
        assert_eq!(loaded.file_size, 1024);
        assert_eq!(loaded.width, Some(640));
    }

    #[test]
    fn insert_and_get_chunk() {
        let db = test_db();
        let post_hash = insert_dummy_post(&db);
        let att = sample_attachment(post_hash);
        insert_media_attachment(&db, &att).unwrap();

        let chunk = StoredChunk {
            chunk_hash: "chunk-aaa".to_string(),
            media_id: "media-001".to_string(),
            chunk_index: 0,
            data: vec![1, 2, 3, 4],
        };
        insert_media_chunk(&db, &chunk).unwrap();

        let loaded = get_media_chunk(&db, "chunk-aaa").unwrap();
        assert_eq!(loaded.data, vec![1, 2, 3, 4]);
        assert_eq!(loaded.chunk_index, 0);
    }

    #[test]
    fn list_media_for_post_returns_all() {
        let db = test_db();
        let post_hash = insert_dummy_post(&db);

        let mut att1 = sample_attachment(post_hash.clone());
        att1.id = "media-001".to_string();
        att1.created_at = 1000;
        insert_media_attachment(&db, &att1).unwrap();

        let mut att2 = sample_attachment(post_hash.clone());
        att2.id = "media-002".to_string();
        att2.content_hash = "def456".to_string();
        att2.created_at = 1001;
        insert_media_attachment(&db, &att2).unwrap();

        let list = list_media_for_post(&db, &post_hash).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "media-001");
        assert_eq!(list[1].id, "media-002");
    }

    #[test]
    fn list_chunks_in_order() {
        let db = test_db();
        let post_hash = insert_dummy_post(&db);
        let att = sample_attachment(post_hash);
        insert_media_attachment(&db, &att).unwrap();

        for i in (0..3).rev() {
            let chunk = StoredChunk {
                chunk_hash: format!("chunk-{i}"),
                media_id: "media-001".to_string(),
                chunk_index: i,
                data: vec![i as u8; 10],
            };
            insert_media_chunk(&db, &chunk).unwrap();
        }

        let chunks = list_chunks_for_media(&db, "media-001").unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[1].chunk_index, 1);
        assert_eq!(chunks[2].chunk_index, 2);
    }

    #[test]
    fn get_thumbnail_hash_returns_value() {
        let db = test_db();
        let post_hash = insert_dummy_post(&db);
        let att = sample_attachment(post_hash);
        insert_media_attachment(&db, &att).unwrap();

        let hash = get_thumbnail_hash(&db, "media-001").unwrap();
        assert_eq!(hash, Some("thumb-hash".to_string()));
    }

    #[test]
    fn missing_attachment_returns_error() {
        let db = test_db();
        let result = get_media_attachment(&db, "nonexistent");
        assert!(result.is_err());
    }
}
