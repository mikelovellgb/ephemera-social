//! Integration tests proving that abuse prevention is wired into the system.
//!
//! Tests:
//! - Rate limiting blocks rapid posting
//! - Spam detection blocks near-duplicates
//! - Content filter blocks spam patterns
//! - Report threshold auto-tombstones content
//! - Report abuse detection deprioritizes prolific reporters
//! - New accounts cannot post media (warming period)
//! - Established accounts can post media

use ephemera_abuse::ReputationScore;
use ephemera_config::NodeConfig;
use ephemera_events::EventBus;
use ephemera_node::services::{MediaFile, ServiceContainer};
use ephemera_types::IdentityKey;
use std::sync::Arc;

/// Helper: create a ServiceContainer backed by a real temp directory.
fn make_services() -> (Arc<ServiceContainer>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = NodeConfig::default_for(dir.path());
    let event_bus = EventBus::new();
    let svc = Arc::new(ServiceContainer::new(&config, event_bus).unwrap());
    (svc, dir)
}

/// Helper: get the current user's IdentityKey from the active identity.
async fn get_identity_key(svc: &ServiceContainer) -> IdentityKey {
    let active = svc.identity.get_active().await.unwrap();
    let pubkey_hex = active["pubkey"].as_str().unwrap();
    let pubkey_bytes = hex::decode(pubkey_hex).unwrap();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&pubkey_bytes);
    IdentityKey::from_bytes(arr)
}

/// Helper: create a text post through the service layer.
async fn create_post(svc: &ServiceContainer, body: &str) -> Result<serde_json::Value, String> {
    svc.posts
        .create(
            body,
            vec![],
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
}

/// Post 11 times quickly; the 11th should fail (rate limit capacity is 10).
#[tokio::test]
async fn test_rate_limit_blocks_rapid_posting() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();

    // Post 10 times (should all succeed -- burst capacity for Post is 10).
    for i in 0..10 {
        let body = format!("Rate limit test post number {i} with unique content");
        let result = create_post(&svc, &body).await;
        assert!(
            result.is_ok(),
            "post {i} should succeed, got: {:?}",
            result.err()
        );
    }

    // The 11th post should be rate-limited.
    let result = create_post(&svc, "This is the 11th post and should be blocked").await;
    assert!(result.is_err(), "11th post should be rate-limited");
    let err = result.unwrap_err();
    assert!(
        err.contains("Rate limited"),
        "error should mention rate limiting: {err}"
    );
}

/// Post the same text twice; the second should be rejected as a near-duplicate.
#[tokio::test]
async fn test_spam_detection_blocks_duplicates() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();

    let body = "This is a duplicate message that should be caught by spam detection";

    // First post should succeed.
    let result1 = create_post(&svc, body).await;
    assert!(result1.is_ok(), "first post should succeed");

    // Second identical post should be rejected.
    let result2 = create_post(&svc, body).await;
    assert!(result2.is_err(), "duplicate post should be rejected");
    let err = result2.unwrap_err();
    assert!(
        err.contains("near-duplicate"),
        "error should mention near-duplicate: {err}"
    );
}

/// Post all-caps text that triggers the content filter.
#[tokio::test]
async fn test_content_filter_blocks_spam_patterns() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();

    // Repeated character spam (>50% of chars are the same).
    let spam_body = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let result = create_post(&svc, spam_body).await;
    assert!(result.is_err(), "repeated char spam should be blocked");
    let err = result.unwrap_err();
    assert!(
        err.contains("content blocked"),
        "error should mention content blocked: {err}"
    );
}

/// 5 reports from different reporters auto-tombstone content.
#[tokio::test]
async fn test_report_threshold_tombstones() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();

    // Create a post to be reported.
    let result = create_post(&svc, "Controversial content for report testing").await.unwrap();
    let content_hash = result["content_hash"].as_str().unwrap().to_string();

    // Verify the post exists.
    let get_result = svc.posts.get(&content_hash, svc.content_store(), &svc.metadata_db).await;
    assert!(get_result.is_ok(), "post should exist before reports");

    // Submit 5 reports from 5 different "reporters" via the seed_report API.
    {
        let hash_bytes = hex::decode(&content_hash).unwrap();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash_bytes);
        let content_id = ephemera_types::ContentId::from_digest(arr);

        for i in 0u8..5 {
            let reporter = IdentityKey::from_bytes([i + 100; 32]);
            svc.moderation
                .seed_report(reporter, content_id.clone(), ephemera_mod::ReportReason::Spam)
                .unwrap();
        }

        // Verify 5 distinct reporters.
        assert_eq!(svc.moderation.reporter_count(&content_id).unwrap(), 5);
    }

    // Now submit a 6th report via the full moderation service.
    // This should trigger the auto-tombstone.
    let report_result = svc
        .moderation
        .report(
            &content_hash,
            "spam",
            &svc.identity,
            &svc.posts,
            &svc.metadata_db,
            &svc.reputation,
        )
        .await
        .unwrap();

    // The response should indicate tombstoning.
    assert!(
        report_result.get("tombstoned").is_some(),
        "report response should indicate tombstoning: {:?}",
        report_result
    );

    // The post should now be a tombstone -- reading it should fail.
    let get_after = svc.posts.get(&content_hash, svc.content_store(), &svc.metadata_db).await;
    assert!(
        get_after.is_err(),
        "post should be tombstoned after report threshold"
    );
}

/// 25 reports from the same user should result in deprioritization.
#[tokio::test]
async fn test_report_abuse_detection() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();
    let identity_key = get_identity_key(&svc).await;

    // Seed 20 reports from the current identity to different content hashes.
    for i in 0u8..20 {
        let content_id = ephemera_types::ContentId::from_digest([i; 32]);
        svc.moderation
            .seed_report(identity_key, content_id, ephemera_mod::ReportReason::Spam)
            .unwrap();
    }

    // The 21st report should be deprioritized.
    let content_hash = hex::encode([0xFFu8; 32]);
    let result = svc
        .moderation
        .report(
            &content_hash,
            "spam",
            &svc.identity,
            &svc.posts,
            &svc.metadata_db,
            &svc.reputation,
        )
        .await
        .unwrap();

    // Should get a "queued for review" response instead of normal report.
    assert_eq!(
        result["status"].as_str(),
        Some("received"),
        "should be deprioritized: {:?}",
        result
    );
    assert!(
        result["note"]
            .as_str()
            .unwrap_or("")
            .contains("queued for review"),
        "should mention queued for review: {:?}",
        result
    );
}

/// A fresh identity should not be able to post media (warming period).
#[tokio::test]
async fn test_new_account_cant_post_media() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();

    // Create a minimal PNG for testing.
    let png_data = {
        let img = image::RgbImage::from_pixel(10, 10, image::Rgb([50, 100, 150]));
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
        buf
    };

    let media_files = vec![MediaFile {
        data: png_data,
        filename: "photo.png".to_string(),
    }];

    let result = svc
        .posts
        .create(
            "Post with media from new account",
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
        .await;

    assert!(result.is_err(), "new account should not be able to post media");
    let err = result.unwrap_err();
    assert!(
        err.contains("too new to post media"),
        "error should mention warming period: {err}"
    );
}

/// An established account (past warming period) should be able to post media.
#[tokio::test]
async fn test_established_account_can_post_media() {
    let (svc, _dir) = make_services();
    svc.identity.create("test-pass").await.unwrap();

    // Warm up the identity: set age past warming period and build reputation.
    let identity_key = get_identity_key(&svc).await;
    {
        let mut rep = ReputationScore::new();
        rep.set_age_days(10); // Past the 7-day warming period
        svc.reputation.lock().unwrap().insert(identity_key, rep);
    }

    // Create a minimal PNG for testing.
    let png_data = {
        let img = image::RgbImage::from_pixel(10, 10, image::Rgb([50, 100, 150]));
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
        buf
    };

    let media_files = vec![MediaFile {
        data: png_data,
        filename: "photo.png".to_string(),
    }];

    let result = svc
        .posts
        .create(
            "Post with media from established account",
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
        .await;

    assert!(
        result.is_ok(),
        "established account should be able to post media: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap()["media_count"].as_i64().unwrap(), 1);
}
