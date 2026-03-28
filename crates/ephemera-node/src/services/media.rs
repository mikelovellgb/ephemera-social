//! Media processing and storage service.
//!
//! Bridges the media processing pipeline (`ephemera-media`) with the
//! storage layer (`ephemera-store`). Handles end-to-end media attachment:
//! detect type, process, chunk, store, and retrieve.

use ephemera_media::chunker::ContentChunker;
use ephemera_media::pipeline::{MediaPipeline, ProcessedContent};
use ephemera_media::video::ProcessedVideo;
use ephemera_media::ProcessedMedia;
use ephemera_store::{
    get_media_attachment, get_media_chunk, get_thumbnail_hash, insert_media_attachment,
    insert_media_chunk, list_media_for_post, ContentStore, MediaAttachment,
    MetadataDb, StoredChunk,
};
use serde_json::Value;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// A media file to be attached to a post.
pub struct MediaFile {
    /// Raw file bytes.
    pub data: Vec<u8>,
    /// Original filename (used for MIME type hint).
    pub filename: String,
}

/// Result of processing and storing a single media file.
#[derive(Debug)]
pub struct StoredMediaResult {
    /// Unique media attachment ID.
    pub media_id: String,
    /// BLAKE3 hex hash of the processed content.
    pub content_hash: String,
    /// "image" or "video".
    pub media_type: String,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64
}

/// Detect MIME type from filename extension.
fn mime_from_filename(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else {
        "application/octet-stream"
    }
}

/// Process a media file through the pipeline, chunk it, and store everything.
///
/// Returns metadata about the stored media.
pub fn process_and_store(
    file: &MediaFile,
    post_hash_wire: &[u8],
    attachment_index: usize,
    content_store: &ContentStore,
    metadata_db: &MetadataDb,
) -> Result<StoredMediaResult, String> {
    let processed = MediaPipeline::process_auto(&file.data)
        .map_err(|e| format!("media processing failed: {e}"))?;

    match processed {
        ProcessedContent::Image(img) => {
            store_image(img, file, post_hash_wire, attachment_index, content_store, metadata_db)
        }
        ProcessedContent::Video(vid) => {
            store_video(vid, file, post_hash_wire, attachment_index, content_store, metadata_db)
        }
    }
}

/// Store a processed image: chunk the standard bytes, store chunks + metadata.
fn store_image(
    img: ProcessedMedia,
    file: &MediaFile,
    post_hash_wire: &[u8],
    attachment_index: usize,
    content_store: &ContentStore,
    metadata_db: &MetadataDb,
) -> Result<StoredMediaResult, String> {
    let content_hash_hex = hex::encode(img.content_hash.hash_bytes());
    let media_id = format!("{content_hash_hex}-{attachment_index}");
    let mime = mime_from_filename(&file.filename);

    // Store thumbnail in content store.
    let thumb_hash = content_store
        .put(&img.thumbnail)
        .map_err(|e| format!("store thumbnail: {e}"))?;

    // Chunk the standard image.
    let chunks = ContentChunker::chunk(&img.standard);

    // Store attachment metadata BEFORE chunks (foreign key constraint).
    let attachment = MediaAttachment {
        id: media_id.clone(),
        post_hash: post_hash_wire.to_vec(),
        content_hash: content_hash_hex.clone(),
        media_type: "image".to_string(),
        mime_type: mime.to_string(),
        file_size: img.standard.len() as i64,
        width: Some(img.standard_width as i64),
        height: Some(img.standard_height as i64),
        duration_ms: None,
        thumbnail_hash: Some(thumb_hash),
        chunk_count: chunks.len() as i64,
        created_at: now_secs(),
    };
    insert_media_attachment(metadata_db, &attachment)
        .map_err(|e| format!("store attachment: {e}"))?;

    // Store each chunk (attachment must exist first).
    for chunk in &chunks {
        let chunk_hash_hex = hex::encode(chunk.hash.hash_bytes());
        let stored = StoredChunk {
            chunk_hash: chunk_hash_hex,
            media_id: media_id.clone(),
            chunk_index: chunk.index as i64,
            data: chunk.data.clone(),
        };
        insert_media_chunk(metadata_db, &stored).map_err(|e| format!("store chunk: {e}"))?;
    }

    Ok(StoredMediaResult {
        media_id,
        content_hash: content_hash_hex,
        media_type: "image".to_string(),
    })
}

/// Store a processed video: chunk the video data, store chunks + metadata.
fn store_video(
    vid: ProcessedVideo,
    _file: &MediaFile,
    post_hash_wire: &[u8],
    attachment_index: usize,
    content_store: &ContentStore,
    metadata_db: &MetadataDb,
) -> Result<StoredMediaResult, String> {
    let content_hash_hex = hex::encode(vid.content_hash.hash_bytes());
    let media_id = format!("{content_hash_hex}-{attachment_index}");

    // Store thumbnail in content store.
    let thumb_hash = content_store
        .put(&vid.thumbnail.thumbnail)
        .map_err(|e| format!("store video thumbnail: {e}"))?;

    // Chunk the video data.
    let chunks = ContentChunker::chunk(&vid.video_data);

    // Store attachment metadata BEFORE chunks (foreign key constraint).
    let attachment = MediaAttachment {
        id: media_id.clone(),
        post_hash: post_hash_wire.to_vec(),
        content_hash: content_hash_hex.clone(),
        media_type: "video".to_string(),
        mime_type: "video/mp4".to_string(),
        file_size: vid.file_size as i64,
        width: Some(vid.width as i64),
        height: Some(vid.height as i64),
        duration_ms: Some(vid.duration_ms as i64),
        thumbnail_hash: Some(thumb_hash),
        chunk_count: chunks.len() as i64,
        created_at: now_secs(),
    };
    insert_media_attachment(metadata_db, &attachment)
        .map_err(|e| format!("store attachment: {e}"))?;

    // Store each chunk (attachment must exist first).
    for chunk in &chunks {
        let chunk_hash_hex = hex::encode(chunk.hash.hash_bytes());
        let stored = StoredChunk {
            chunk_hash: chunk_hash_hex,
            media_id: media_id.clone(),
            chunk_index: chunk.index as i64,
            data: chunk.data.clone(),
        };
        insert_media_chunk(metadata_db, &stored).map_err(|e| format!("store chunk: {e}"))?;
    }

    Ok(StoredMediaResult {
        media_id,
        content_hash: content_hash_hex,
        media_type: "video".to_string(),
    })
}

/// Retrieve metadata for a media attachment.
pub fn get_metadata(
    media_id: &str,
    metadata_db: &Mutex<MetadataDb>,
) -> Result<Value, String> {
    let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
    let att = get_media_attachment(&db, media_id)
        .map_err(|e| format!("media not found: {e}"))?;

    Ok(serde_json::json!({
        "id": att.id,
        "content_hash": att.content_hash,
        "media_type": att.media_type,
        "mime_type": att.mime_type,
        "file_size": att.file_size,
        "width": att.width,
        "height": att.height,
        "duration_ms": att.duration_ms,
        "chunk_count": att.chunk_count,
    }))
}

/// Retrieve a single chunk's data by its hash.
pub fn get_chunk(
    chunk_hash: &str,
    metadata_db: &Mutex<MetadataDb>,
) -> Result<Value, String> {
    let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
    let chunk = get_media_chunk(&db, chunk_hash)
        .map_err(|e| format!("chunk not found: {e}"))?;

    // Return hex-encoded data for JSON transport.
    let encoded = hex::encode(&chunk.data);
    Ok(serde_json::json!({
        "chunk_hash": chunk.chunk_hash,
        "media_id": chunk.media_id,
        "chunk_index": chunk.chunk_index,
        "data_hex": encoded,
        "size": chunk.data.len(),
    }))
}

/// Retrieve the thumbnail for a media attachment.
pub fn get_thumbnail(
    media_id: &str,
    metadata_db: &Mutex<MetadataDb>,
    content_store: &ContentStore,
) -> Result<Value, String> {
    let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
    let thumb_hash = get_thumbnail_hash(&db, media_id)
        .map_err(|e| format!("media not found: {e}"))?;

    match thumb_hash {
        Some(hash) => {
            drop(db);
            let data = content_store
                .get(&hash)
                .map_err(|e| format!("thumbnail blob not found: {e}"))?;
            let encoded = hex::encode(&data);
            Ok(serde_json::json!({
                "media_id": media_id,
                "thumbnail_hash": hash,
                "data_hex": encoded,
                "size": data.len(),
            }))
        }
        None => Err(format!("no thumbnail for media {media_id}")),
    }
}

/// List all media attachments for a post (by hex content hash).
pub fn list_for_post(
    post_hash_hex: &str,
    metadata_db: &Mutex<MetadataDb>,
) -> Result<Value, String> {
    let hash_bytes = hex::decode(post_hash_hex).map_err(|e| format!("bad hash: {e}"))?;
    if hash_bytes.len() != 32 {
        return Err("content hash must be 32 bytes hex".into());
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hash_bytes);
    let content_id = ephemera_types::ContentId::from_digest(arr);
    let wire = content_id.to_wire_bytes();

    let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
    let attachments = list_media_for_post(&db, &wire)
        .map_err(|e| format!("list media: {e}"))?;

    let items: Vec<Value> = attachments
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "content_hash": a.content_hash,
                "media_type": a.media_type,
                "mime_type": a.mime_type,
                "file_size": a.file_size,
                "width": a.width,
                "height": a.height,
                "duration_ms": a.duration_ms,
                "chunk_count": a.chunk_count,
            })
        })
        .collect();

    Ok(serde_json::json!({ "attachments": items }))
}
