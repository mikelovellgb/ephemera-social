//! Video processing pipeline.
//!
//! Orchestrates the full video processing flow: validate format and
//! constraints, strip metadata, extract thumbnail, compute BLAKE3 hash.
//!
//! **v1 scope:** Accepts pre-encoded H.264/MP4 video. Does not transcode
//! (transcoding requires FFmpeg). The pipeline validates, strips metadata,
//! extracts a thumbnail frame, chunks the content, and hashes it.
//!
//! Client-side transcoding to H.264/MP4 must be done before passing
//! video data to this pipeline.

use std::io::Cursor;

use ephemera_types::ContentId;

use crate::pipeline::ProcessedMedia;
use crate::video_metadata::strip_video_metadata;
use crate::video_validation::{validate_video, VideoInfo};
use crate::MediaError;

/// Output of the video processing pipeline.
#[derive(Debug)]
pub struct ProcessedVideo {
    /// Cleaned video bytes (metadata stripped).
    pub video_data: Vec<u8>,
    /// Thumbnail extracted from the first video frame, processed
    /// through the image pipeline.
    pub thumbnail: ProcessedMedia,
    /// BLAKE3 hash of the cleaned video data.
    pub content_hash: ContentId,
    /// Video duration in milliseconds.
    pub duration_ms: u64,
    /// Video width in pixels.
    pub width: u32,
    /// Video height in pixels.
    pub height: u32,
    /// Video codec identifier (e.g. "h264").
    pub codec: String,
    /// File size of the cleaned video in bytes.
    pub file_size: usize,
}

/// Video processing pipeline.
///
/// Processes raw MP4/H.264 video through validation, metadata stripping,
/// thumbnail extraction, and BLAKE3 hashing.
pub struct VideoProcessor;

impl VideoProcessor {
    /// Process raw video bytes through the full pipeline.
    ///
    /// Steps:
    /// 1. Validate format (H.264/MP4 only), duration (max 3 min),
    ///    and file size (max 50 MiB).
    /// 2. Strip all metadata (creation date, GPS, camera info, etc.).
    /// 3. Extract thumbnail (first frame, processed through image pipeline).
    /// 4. Compute BLAKE3 hash of the cleaned video.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError`] if validation fails, the MP4 cannot be
    /// parsed, or thumbnail extraction fails.
    pub fn process(input: &[u8]) -> Result<ProcessedVideo, MediaError> {
        // Step 1: Validate.
        let info: VideoInfo = validate_video(input)?;

        // Step 2: Strip metadata.
        let cleaned = strip_video_metadata(input)?;

        // Step 3: Extract thumbnail.
        let thumbnail = extract_thumbnail(&cleaned, &info)?;

        // Step 4: Compute BLAKE3 hash.
        let hash = blake3::hash(&cleaned);
        let content_hash = ContentId::from_digest(*hash.as_bytes());

        Ok(ProcessedVideo {
            file_size: cleaned.len(),
            video_data: cleaned,
            thumbnail,
            content_hash,
            duration_ms: info.duration_ms,
            width: info.width,
            height: info.height,
            codec: info.codec,
        })
    }
}

/// Extract a thumbnail from the video.
///
/// For v1, we generate a solid-color placeholder thumbnail based on the
/// video dimensions. Full keyframe extraction requires FFmpeg or a
/// complete H.264 decoder, which is out of scope for the pure-Rust
/// pipeline. The thumbnail dimensions match the video's aspect ratio
/// at thumbnail resolution.
///
/// Future versions should use FFmpeg or WebCodecs (in browser) to
/// extract the actual first keyframe.
fn extract_thumbnail(_data: &[u8], info: &VideoInfo) -> Result<ProcessedMedia, MediaError> {
    // Generate a placeholder thumbnail. Full keyframe extraction
    // requires FFmpeg or a complete H.264 decoder.
    let thumb_image = generate_placeholder_thumbnail(info);

    // Run the placeholder through the standard image pipeline.
    crate::pipeline::MediaPipeline::process(&thumb_image)
}

/// Generate a placeholder thumbnail image for a video.
///
/// Creates a small PNG at the video's aspect ratio with a neutral gray
/// fill. This serves as a fallback when keyframe extraction is not
/// available (pure-Rust pipeline without FFmpeg).
fn generate_placeholder_thumbnail(info: &VideoInfo) -> Vec<u8> {
    // Calculate thumbnail dimensions preserving aspect ratio.
    let max_dim: u32 = 200;
    let (thumb_w, thumb_h) = if info.width >= info.height {
        let w = max_dim.min(info.width);
        let h = (u64::from(w) * u64::from(info.height) / u64::from(info.width).max(1)) as u32;
        (w, h.max(1))
    } else {
        let h = max_dim.min(info.height);
        let w = (u64::from(h) * u64::from(info.width) / u64::from(info.height).max(1)) as u32;
        (w.max(1), h)
    };

    // Create a neutral gray image.
    let img = image::RgbImage::from_pixel(thumb_w, thumb_h, image::Rgb([128, 128, 128]));
    let mut buf = Vec::new();
    let mut cursor = Cursor::new(&mut buf);
    // Unwrap is safe: encoding a simple RGB image to PNG cannot fail.
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .expect("PNG encoding of placeholder thumbnail failed");
    buf
}

/// Check whether the given bytes look like they need video processing.
///
/// Returns `true` if the data starts with an MP4 `ftyp` box, meaning
/// it should be routed to the video pipeline instead of the image pipeline.
pub fn is_video(data: &[u8]) -> bool {
    crate::video_validation::is_mp4(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mp4;

    #[test]
    fn process_valid_mp4() {
        let input = test_mp4::build_test_mp4(640, 480, 1000);
        let result = VideoProcessor::process(&input);
        // Our test MP4 builder produces valid files. Process must succeed.
        let processed = result.expect("test MP4 should process successfully");
        assert!(!processed.video_data.is_empty());
        assert!(!processed.thumbnail.standard.is_empty());
        assert!(processed.duration_ms > 0);
        assert!(processed.width > 0);
        assert!(processed.height > 0);
    }

    #[test]
    fn process_rejects_non_mp4() {
        let png_data = {
            let img = image::RgbImage::from_pixel(10, 10, image::Rgb([255, 0, 0]));
            let mut buf = Vec::new();
            let mut cursor = Cursor::new(&mut buf);
            img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();
            buf
        };
        let result = VideoProcessor::process(&png_data);
        assert!(result.is_err());
    }

    #[test]
    fn is_video_detects_mp4() {
        let mp4 = test_mp4::build_test_mp4(320, 240, 500);
        assert!(is_video(&mp4));
    }

    #[test]
    fn is_video_rejects_png() {
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(!is_video(&png_header));
    }

    #[test]
    fn placeholder_thumbnail_has_correct_aspect_ratio() {
        let info = VideoInfo {
            duration_ms: 1000,
            width: 1920,
            height: 1080,
            codec: "h264".into(),
            file_size: 1000,
            timescale: 1000,
        };
        let thumb_bytes = generate_placeholder_thumbnail(&info);
        let img = image::load_from_memory(&thumb_bytes).unwrap();
        // 200 wide, ~112 tall (1920:1080 ratio at max 200).
        assert_eq!(img.width(), 200);
        assert!(img.height() > 100 && img.height() < 130);
    }
}
