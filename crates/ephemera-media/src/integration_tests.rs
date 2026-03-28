//! Integration tests for the complete video processing pipeline.
//!
//! These tests exercise the full pipeline end-to-end, testing
//! interactions between validation, metadata stripping, thumbnail
//! extraction, chunking, and hashing.

#[cfg(test)]
mod tests {
    use crate::chunker::ContentChunker;
    use crate::pipeline::MediaPipeline;
    use crate::test_mp4;
    use crate::video::VideoProcessor;
    use crate::video_metadata::{strip_video_metadata, verify_metadata_stripped};
    use crate::video_validation::{validate_video, MAX_VIDEO_DURATION_MS, MAX_VIDEO_INPUT_SIZE};

    // ---- Video validation tests ----

    #[test]
    fn test_video_validation_rejects_oversized() {
        // 51 MiB should fail (limit is 50 MiB).
        let target = MAX_VIDEO_INPUT_SIZE + 1;
        // Build a minimal MP4 with ftyp header, then pad it.
        let mut data = vec![0u8; target];
        data[0..4].copy_from_slice(&8u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        let result = validate_video(&data);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("exceeds maximum"),
            "expected size error, got: {err_msg}"
        );
    }

    #[test]
    fn test_video_validation_rejects_long_duration() {
        // >3 min should fail.
        let input = test_mp4::build_long_duration_mp4(MAX_VIDEO_DURATION_MS + 1000);
        let result = validate_video(&input);
        // The result MUST be an error -- either a duration rejection or a
        // parse error. Accepting the video as valid is never correct.
        assert!(
            result.is_err(),
            "over-duration video must be rejected, but got Ok({:?})",
            result.unwrap()
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("duration") || msg.contains("parse") || msg.contains("MP4"),
            "expected duration or parse error, got: {msg}"
        );
    }

    #[test]
    fn test_video_validation_accepts_valid() {
        let input = test_mp4::build_test_mp4(640, 480, 5000);
        let result = validate_video(&input);
        // Our test MP4 builder produces valid files. If this fails, the
        // builder is broken and must be fixed -- not silently ignored.
        let info = result.expect("test MP4 should validate successfully");
        assert_eq!(info.width, 640);
        assert_eq!(info.height, 480);
        assert!(info.duration_ms > 0);
        assert!(!info.codec.is_empty());
    }

    // ---- Thumbnail extraction tests ----

    #[test]
    fn test_thumbnail_extraction() {
        let input = test_mp4::build_test_mp4(640, 480, 1000);
        let result = VideoProcessor::process(&input);
        // Our test MP4 builder produces valid files. Process must succeed.
        let processed = result.expect("test MP4 should process successfully");
        let thumb = &processed.thumbnail;
        assert!(!thumb.standard.is_empty());
        assert!(!thumb.thumbnail.is_empty());
        // Verify it decodes as an image.
        let img = image::load_from_memory(&thumb.standard);
        assert!(img.is_ok(), "thumbnail should be a valid image");
    }

    // ---- Metadata stripping tests ----

    #[test]
    fn test_metadata_stripped() {
        // Build an MP4 with metadata in a udta box.
        let mut data = Vec::new();

        // ftyp box.
        let ftyp_size = 20u32;
        data.extend_from_slice(&ftyp_size.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(b"isom");
        data.extend_from_slice(&[0u8; 8]);

        // moov box with udta containing fake GPS data.
        let gps_data = b"GPS:47.6062N,122.3321W";
        let udta_size = (8 + gps_data.len()) as u32;
        let moov_size = 8 + udta_size;
        data.extend_from_slice(&moov_size.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.extend_from_slice(&udta_size.to_be_bytes());
        data.extend_from_slice(b"udta");
        data.extend_from_slice(gps_data);

        // Verify metadata is present before stripping.
        assert!(
            !verify_metadata_stripped(&data),
            "should detect metadata before stripping"
        );

        // Strip metadata.
        let stripped = strip_video_metadata(&data).unwrap();

        // Verify metadata is gone after stripping.
        assert!(
            verify_metadata_stripped(&stripped),
            "metadata should be stripped"
        );

        // Verify the GPS string is no longer present.
        let stripped_str = String::from_utf8_lossy(&stripped);
        assert!(
            !stripped_str.contains("GPS:47.6062N"),
            "GPS data should be removed"
        );
    }

    // ---- Chunking integration tests ----

    #[test]
    fn test_chunk_and_reassemble() {
        let data: Vec<u8> = (0u8..=255).cycle().take(700_000).collect();
        let chunks = ContentChunker::chunk(&data);

        // 700,000 / 262,144 = 2 full + 1 partial = 3 chunks.
        assert_eq!(chunks.len(), 3);

        let reassembled = ContentChunker::reassemble(&chunks).unwrap();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn test_chunk_hash_verification() {
        let data: Vec<u8> = (0u8..=255).cycle().take(700_000).collect();
        let mut chunks = ContentChunker::chunk(&data);

        // Tamper with the middle chunk.
        chunks[1].data[0] ^= 0xFF;

        let result = ContentChunker::reassemble(&chunks);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tampered"));
    }

    #[test]
    fn test_chunk_image_and_reassemble() {
        // Process an image, then chunk and reassemble.
        let img = image::RgbImage::from_pixel(100, 100, image::Rgb([50, 100, 150]));
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();

        let processed = MediaPipeline::process(&buf).unwrap();
        let chunks = ContentChunker::chunk(&processed.standard);
        let reassembled = ContentChunker::reassemble(&chunks).unwrap();
        assert_eq!(reassembled, processed.standard);
    }

    #[test]
    fn test_video_content_hash_is_deterministic() {
        let input = test_mp4::build_test_mp4(640, 480, 1000);
        let p1 = VideoProcessor::process(&input).expect("first process should succeed");
        let p2 = VideoProcessor::process(&input).expect("second process should succeed");
        assert_eq!(
            p1.content_hash, p2.content_hash,
            "deterministic processing must produce the same content hash"
        );
    }
}
