//! Media processing pipeline.
//!
//! Orchestrates the full media processing flow: validate -> strip EXIF ->
//! resize -> compress -> hash. Supports both images and video — input is
//! routed automatically based on magic-byte detection.

use ephemera_types::ContentId;
use image::ImageFormat;

use crate::exif::strip_exif_data;
use crate::resize::{
    generate_thumbnail, resize_image, ResizeResult, STANDARD_MAX_HEIGHT, STANDARD_MAX_WIDTH,
};
use crate::validation::{validate_media, SupportedFormat};
use crate::video;
use crate::MediaError;

/// The output of the image processing pipeline.
#[derive(Debug)]
pub struct ProcessedMedia {
    /// The processed standard-resolution image bytes.
    pub standard: Vec<u8>,
    /// Width of the standard image.
    pub standard_width: u32,
    /// Height of the standard image.
    pub standard_height: u32,
    /// The thumbnail image bytes.
    pub thumbnail: Vec<u8>,
    /// BLAKE3 hash of the standard image (content address).
    pub content_hash: ContentId,
    /// Detected format of the input.
    pub original_format: SupportedFormat,
}

/// Unified result that can hold either processed image or video data.
#[derive(Debug)]
pub enum ProcessedContent {
    /// The input was an image.
    Image(ProcessedMedia),
    /// The input was a video.
    Video(video::ProcessedVideo),
}

/// The media processing pipeline.
///
/// Processes raw media bytes through validation, metadata stripping,
/// resizing/processing, and hashing. Automatically detects whether the
/// input is an image or video and routes accordingly.
pub struct MediaPipeline;

impl MediaPipeline {
    /// Process raw image bytes through the full image pipeline.
    ///
    /// Steps:
    /// 1. Validate file type and size.
    /// 2. Strip EXIF/GPS metadata.
    /// 3. Resize to standard dimensions (max 1920x1080).
    /// 4. Generate thumbnail (200x200).
    /// 5. Compute BLAKE3 content hash.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError`] if any step fails.
    pub fn process(raw_bytes: &[u8]) -> Result<ProcessedMedia, MediaError> {
        // Step 1: Validate.
        let format = validate_media(raw_bytes)?;
        let image_format = to_image_format(format);

        // Step 2: Strip EXIF metadata.
        let stripped = strip_exif_data(raw_bytes, image_format)?;

        // Step 3: Resize to standard dimensions.
        let ResizeResult {
            data: standard,
            width: std_w,
            height: std_h,
        } = resize_image(&stripped, STANDARD_MAX_WIDTH, STANDARD_MAX_HEIGHT)?;

        // Step 4: Generate thumbnail.
        let ResizeResult {
            data: thumbnail, ..
        } = generate_thumbnail(&stripped)?;

        // Step 5: Hash the standard image.
        let hash = blake3::hash(&standard);
        let content_hash = ContentId::from_digest(*hash.as_bytes());

        Ok(ProcessedMedia {
            standard,
            standard_width: std_w,
            standard_height: std_h,
            thumbnail,
            content_hash,
            original_format: format,
        })
    }

    /// Auto-detect content type and process through the appropriate pipeline.
    ///
    /// Examines the magic bytes of the input:
    /// - MP4 (`ftyp` box): routes to the video pipeline
    /// - JPEG/PNG/WebP: routes to the image pipeline
    ///
    /// # Errors
    ///
    /// Returns [`MediaError`] if the format is unrecognized or processing fails.
    pub fn process_auto(raw_bytes: &[u8]) -> Result<ProcessedContent, MediaError> {
        if video::is_video(raw_bytes) {
            let processed = video::VideoProcessor::process(raw_bytes)?;
            Ok(ProcessedContent::Video(processed))
        } else {
            let processed = Self::process(raw_bytes)?;
            Ok(ProcessedContent::Image(processed))
        }
    }
}

/// Convert our `SupportedFormat` to an `image` crate `ImageFormat`.
fn to_image_format(format: SupportedFormat) -> ImageFormat {
    match format {
        SupportedFormat::Jpeg => ImageFormat::Jpeg,
        SupportedFormat::Png => ImageFormat::Png,
        SupportedFormat::WebP => ImageFormat::WebP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([50, 100, 150]));
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn pipeline_processes_png() {
        let input = make_test_png(640, 480);
        let result = MediaPipeline::process(&input).unwrap();

        assert_eq!(result.original_format, SupportedFormat::Png);
        assert_eq!(result.standard_width, 640);
        assert_eq!(result.standard_height, 480);
        assert!(!result.standard.is_empty());
        assert!(!result.thumbnail.is_empty());
    }

    #[test]
    fn pipeline_resizes_large_image() {
        let input = make_test_png(4000, 3000);
        let result = MediaPipeline::process(&input).unwrap();

        assert!(result.standard_width <= STANDARD_MAX_WIDTH);
        assert!(result.standard_height <= STANDARD_MAX_HEIGHT);
    }

    #[test]
    fn pipeline_rejects_garbage() {
        let garbage = vec![0x00; 1024];
        assert!(MediaPipeline::process(&garbage).is_err());
    }

    #[test]
    fn pipeline_content_hash_is_deterministic() {
        let input = make_test_png(100, 100);
        let r1 = MediaPipeline::process(&input).unwrap();
        let r2 = MediaPipeline::process(&input).unwrap();
        assert_eq!(r1.content_hash, r2.content_hash);
    }

    #[test]
    fn process_auto_routes_png_to_image() {
        let input = make_test_png(100, 100);
        let result = MediaPipeline::process_auto(&input).unwrap();
        assert!(matches!(result, ProcessedContent::Image(_)));
    }

    #[test]
    fn process_auto_routes_mp4_to_video() {
        let input = crate::test_mp4::build_test_mp4(640, 480, 1000);
        let result = MediaPipeline::process_auto(&input);
        // Our test MP4 builder produces valid files. Routing must succeed.
        match result {
            Ok(ProcessedContent::Video(_)) => {} // Expected route.
            Ok(ProcessedContent::Image(_)) => panic!("MP4 should not route to image pipeline"),
            Err(e) => panic!("MP4 routing should succeed, got error: {e}"),
        }
    }

    #[test]
    fn process_auto_rejects_garbage() {
        let garbage = vec![0x00; 1024];
        assert!(MediaPipeline::process_auto(&garbage).is_err());
    }
}
