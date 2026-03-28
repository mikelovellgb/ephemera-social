//! Integration tests for the media pipeline wired into post creation.
//!
//! Tests end-to-end: create a post with media attachments, verify
//! media is processed, chunked, stored, and retrievable.

use ephemera_abuse::ReputationScore;
use ephemera_config::NodeConfig;
use ephemera_events::EventBus;
use ephemera_node::services::{MediaFile, ServiceContainer};
use ephemera_store::{get_media_attachment, list_chunks_for_media, list_media_for_post};
use std::sync::Arc;

/// Helper: create a ServiceContainer backed by a real temp directory.
fn make_services() -> (Arc<ServiceContainer>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let event_bus = EventBus::new();
    let svc = Arc::new(ServiceContainer::new(&config, event_bus).unwrap());
    (svc, dir)
}

/// Warm up the identity in the reputation tracker so media posting is allowed.
async fn warm_up_identity(svc: &ServiceContainer) {
    let active = svc.identity.get_active().await.unwrap();
    let pubkey_hex = active["pubkey"].as_str().unwrap();
    let pubkey_bytes = hex::decode(pubkey_hex).unwrap();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&pubkey_bytes);
    let identity_key = ephemera_types::IdentityKey::from_bytes(arr);
    let mut rep = ReputationScore::new();
    rep.set_age_days(10);
    svc.reputation.lock().unwrap().insert(identity_key, rep);
}

/// Create a minimal valid PNG image for testing.
fn make_test_png(width: u32, height: u32) -> Vec<u8> {
    let img =
        image::RgbImage::from_pixel(width, height, image::Rgb([50, 100, 150]));
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
    buf
}

/// Test: create a post with a PNG image, verify media stored correctly.
#[tokio::test]
async fn test_create_post_with_image() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();
    warm_up_identity(&svc).await;

    let png_data = make_test_png(640, 480);
    let media_files = vec![MediaFile {
        data: png_data.clone(),
        filename: "photo.png".to_string(),
    }];

    let result = svc
        .posts
        .create(
            "Check out this photo!",
            media_files,
            Some(86400),
            None,
            &svc.identity,
            svc.content_store(),
            &svc.metadata_db,
            &svc.rate_limiter,
            &svc.reputation,
            &svc.fingerprint_store,
            &svc.content_filter,
            None,
        )
        .await
        .unwrap();

    // Post should report 1 media attachment.
    assert_eq!(result["media_count"].as_i64().unwrap(), 1);

    let content_hash = result["content_hash"].as_str().unwrap();
    assert_eq!(content_hash.len(), 64);

    // Verify media metadata was stored in the database.
    let hash_bytes = hex::decode(content_hash).unwrap();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hash_bytes);
    let content_id = ephemera_types::ContentId::from_digest(arr);
    let wire = content_id.to_wire_bytes();

    let db = svc.metadata_db.lock().unwrap();
    let attachments = list_media_for_post(&db, &wire).unwrap();
    assert_eq!(attachments.len(), 1);

    let att = &attachments[0];
    assert_eq!(att.media_type, "image");
    assert_eq!(att.mime_type, "image/png");
    assert!(att.file_size > 0);
    assert_eq!(att.width, Some(640));
    assert_eq!(att.height, Some(480));
    assert!(att.thumbnail_hash.is_some());
    assert!(att.chunk_count >= 1);

    // Verify chunks exist.
    let chunks = list_chunks_for_media(&db, &att.id).unwrap();
    assert_eq!(chunks.len() as i64, att.chunk_count);
    assert!(!chunks[0].data.is_empty());
}

/// Test: create a post with an MP4 video, verify chunks stored.
#[tokio::test]
async fn test_create_post_with_video() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();
    warm_up_identity(&svc).await;

    // Build a test MP4 file using the media crate's test builder.
    let mp4_data = ephemera_media::test_mp4::build_test_mp4(320, 240, 1000);
    let media_files = vec![MediaFile {
        data: mp4_data,
        filename: "clip.mp4".to_string(),
    }];

    let result = svc
        .posts
        .create(
            "Watch this video!",
            media_files,
            Some(86400),
            None,
            &svc.identity,
            svc.content_store(),
            &svc.metadata_db,
            &svc.rate_limiter,
            &svc.reputation,
            &svc.fingerprint_store,
            &svc.content_filter,
            None,
        )
        .await
        .unwrap();

    assert_eq!(result["media_count"].as_i64().unwrap(), 1);

    let content_hash = result["content_hash"].as_str().unwrap();
    let hash_bytes = hex::decode(content_hash).unwrap();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hash_bytes);
    let content_id = ephemera_types::ContentId::from_digest(arr);
    let wire = content_id.to_wire_bytes();

    let db = svc.metadata_db.lock().unwrap();
    let attachments = list_media_for_post(&db, &wire).unwrap();
    assert_eq!(attachments.len(), 1);

    let att = &attachments[0];
    assert_eq!(att.media_type, "video");
    assert_eq!(att.mime_type, "video/mp4");
    assert!(att.file_size > 0);
    assert!(att.width.unwrap() > 0);
    assert!(att.height.unwrap() > 0);
    assert!(att.duration_ms.unwrap() > 0);
    assert!(att.thumbnail_hash.is_some());

    // Verify all chunks exist.
    let chunks = list_chunks_for_media(&db, &att.id).unwrap();
    assert_eq!(chunks.len() as i64, att.chunk_count);
    for chunk in &chunks {
        assert!(!chunk.data.is_empty());
    }
}

/// Test: store media via post creation, then retrieve it via the media service API.
#[tokio::test]
async fn test_media_retrieval() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();
    warm_up_identity(&svc).await;

    let png_data = make_test_png(100, 100);
    let media_files = vec![MediaFile {
        data: png_data,
        filename: "small.png".to_string(),
    }];

    let result = svc
        .posts
        .create(
            "Small image post",
            media_files,
            Some(86400),
            None,
            &svc.identity,
            svc.content_store(),
            &svc.metadata_db,
            &svc.rate_limiter,
            &svc.reputation,
            &svc.fingerprint_store,
            &svc.content_filter,
            None,
        )
        .await
        .unwrap();

    let content_hash = result["content_hash"].as_str().unwrap();

    // Get media metadata via the service function.
    let media_list = ephemera_node::services::media::list_for_post(
        content_hash,
        &svc.metadata_db,
    )
    .unwrap();

    let attachments = media_list["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 1);

    let media_id = attachments[0]["id"].as_str().unwrap();

    // Retrieve metadata.
    let meta = ephemera_node::services::media::get_metadata(
        media_id,
        &svc.metadata_db,
    )
    .unwrap();
    assert_eq!(meta["media_type"].as_str().unwrap(), "image");
    assert!(meta["file_size"].as_i64().unwrap() > 0);

    // Retrieve the first chunk.
    let db = svc.metadata_db.lock().unwrap();
    let chunks = list_chunks_for_media(&db, media_id).unwrap();
    drop(db);

    assert!(!chunks.is_empty());
    let first_chunk_hash = &chunks[0].chunk_hash;

    let chunk_data = ephemera_node::services::media::get_chunk(
        first_chunk_hash,
        &svc.metadata_db,
    )
    .unwrap();
    assert!(chunk_data["size"].as_u64().unwrap() > 0);

    // Retrieve thumbnail.
    let thumbnail = ephemera_node::services::media::get_thumbnail(
        media_id,
        &svc.metadata_db,
        svc.content_store(),
    )
    .unwrap();
    assert!(thumbnail["size"].as_u64().unwrap() > 0);
}

/// Test: chunk integrity — store media, reassemble all chunks, verify
/// BLAKE3 hash matches the original processed content hash.
#[tokio::test]
async fn test_media_chunks_integrity() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();
    warm_up_identity(&svc).await;

    // Use a large enough image that it creates multiple chunks (>256KB).
    let png_data = make_test_png(1920, 1080);
    let media_files = vec![MediaFile {
        data: png_data,
        filename: "big.png".to_string(),
    }];

    let result = svc
        .posts
        .create(
            "Big image",
            media_files,
            Some(86400),
            None,
            &svc.identity,
            svc.content_store(),
            &svc.metadata_db,
            &svc.rate_limiter,
            &svc.reputation,
            &svc.fingerprint_store,
            &svc.content_filter,
            None,
        )
        .await
        .unwrap();

    let content_hash = result["content_hash"].as_str().unwrap();
    let hash_bytes = hex::decode(content_hash).unwrap();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&hash_bytes);
    let content_id = ephemera_types::ContentId::from_digest(arr);
    let wire = content_id.to_wire_bytes();

    let db = svc.metadata_db.lock().unwrap();
    let attachments = list_media_for_post(&db, &wire).unwrap();
    assert_eq!(attachments.len(), 1);

    let att = &attachments[0];
    let stored_content_hash = &att.content_hash;

    // Reassemble chunks.
    let chunks = list_chunks_for_media(&db, &att.id).unwrap();

    // Verify chunk indices are contiguous.
    for (i, chunk) in chunks.iter().enumerate() {
        assert_eq!(chunk.chunk_index, i as i64, "chunk index mismatch");
    }

    // Reassemble and verify BLAKE3 hash.
    let mut reassembled = Vec::new();
    for chunk in &chunks {
        reassembled.extend_from_slice(&chunk.data);
    }

    let computed_hash = blake3::hash(&reassembled);
    let computed_hex = computed_hash.to_hex().to_string();

    assert_eq!(
        computed_hex, *stored_content_hash,
        "reassembled content hash should match stored content hash"
    );

    // Verify each individual chunk hash.
    for chunk in &chunks {
        let expected = blake3::hash(&chunk.data).to_hex().to_string();
        assert_eq!(
            chunk.chunk_hash, expected,
            "individual chunk hash should match data"
        );
    }
}
