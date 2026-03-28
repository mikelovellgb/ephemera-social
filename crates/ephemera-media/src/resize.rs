//! Image resizing.
//!
//! Resizes images to standard and thumbnail dimensions while preserving
//! aspect ratio. Uses Lanczos3 resampling for high quality.

use image::{DynamicImage, ImageFormat};

use crate::MediaError;

/// Standard maximum width for display images.
pub const STANDARD_MAX_WIDTH: u32 = 1920;

/// Standard maximum height for display images.
pub const STANDARD_MAX_HEIGHT: u32 = 1080;

/// Thumbnail width.
pub const THUMBNAIL_WIDTH: u32 = 200;

/// Thumbnail height.
pub const THUMBNAIL_HEIGHT: u32 = 200;

/// Resize result containing the resized image bytes.
#[derive(Debug)]
pub struct ResizeResult {
    /// The resized image bytes (PNG-encoded).
    pub data: Vec<u8>,
    /// Final width.
    pub width: u32,
    /// Final height.
    pub height: u32,
}

/// Resize an image to fit within the given maximum dimensions.
///
/// Preserves aspect ratio. If the image is already smaller than the
/// maximum dimensions, it is returned unchanged.
///
/// # Errors
///
/// Returns [`MediaError`] if the image cannot be decoded or re-encoded.
pub fn resize_image(
    data: &[u8],
    max_width: u32,
    max_height: u32,
) -> Result<ResizeResult, MediaError> {
    let img = image::load_from_memory(data)
        .map_err(|e| MediaError::Processing(format!("failed to decode for resize: {e}")))?;

    let (orig_w, orig_h) = (img.width(), img.height());

    // Only resize if the image exceeds the maximum dimensions.
    let resized = if orig_w > max_width || orig_h > max_height {
        img.resize(max_width, max_height, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    let (final_w, final_h) = (resized.width(), resized.height());
    let encoded = encode_png(&resized)?;

    Ok(ResizeResult {
        data: encoded,
        width: final_w,
        height: final_h,
    })
}

/// Generate a square thumbnail by cropping to center and resizing.
///
/// # Errors
///
/// Returns [`MediaError`] if the image cannot be decoded or re-encoded.
pub fn generate_thumbnail(data: &[u8]) -> Result<ResizeResult, MediaError> {
    let img = image::load_from_memory(data)
        .map_err(|e| MediaError::Processing(format!("failed to decode for thumbnail: {e}")))?;

    let thumb = img.resize_to_fill(
        THUMBNAIL_WIDTH,
        THUMBNAIL_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );

    let encoded = encode_png(&thumb)?;

    Ok(ResizeResult {
        data: encoded,
        width: thumb.width(),
        height: thumb.height(),
    })
}

/// Encode a `DynamicImage` to PNG bytes.
fn encode_png(img: &DynamicImage) -> Result<Vec<u8>, MediaError> {
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    img.write_to(&mut cursor, ImageFormat::Png)
        .map_err(|e| MediaError::Processing(format!("PNG encoding failed: {e}")))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_image(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([100, 150, 200]));
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn resize_within_limits_is_noop() {
        let data = make_test_image(100, 100);
        let result = resize_image(&data, STANDARD_MAX_WIDTH, STANDARD_MAX_HEIGHT).unwrap();
        assert_eq!(result.width, 100);
        assert_eq!(result.height, 100);
    }

    #[test]
    fn resize_large_image() {
        let data = make_test_image(4000, 3000);
        let result = resize_image(&data, STANDARD_MAX_WIDTH, STANDARD_MAX_HEIGHT).unwrap();
        assert!(result.width <= STANDARD_MAX_WIDTH);
        assert!(result.height <= STANDARD_MAX_HEIGHT);
    }

    #[test]
    fn thumbnail_dimensions() {
        let data = make_test_image(800, 600);
        let thumb = generate_thumbnail(&data).unwrap();
        assert_eq!(thumb.width, THUMBNAIL_WIDTH);
        assert_eq!(thumb.height, THUMBNAIL_HEIGHT);
    }

    #[test]
    fn resize_preserves_aspect_ratio() {
        let data = make_test_image(4000, 2000); // 2:1 ratio
        let result = resize_image(&data, 1920, 1080).unwrap();
        // At 1920 wide, height should be 960 (maintains 2:1).
        assert_eq!(result.width, 1920);
        assert_eq!(result.height, 960);
    }

    #[test]
    fn resize_invalid_data_fails() {
        let garbage = vec![0x00; 100];
        assert!(resize_image(&garbage, 1920, 1080).is_err());
    }
}
