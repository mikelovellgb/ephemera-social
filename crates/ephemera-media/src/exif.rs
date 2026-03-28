//! EXIF metadata stripping.
//!
//! Removes all EXIF, IPTC, and XMP metadata from images to prevent
//! leaking GPS coordinates, camera model, timestamps, and other
//! personally identifiable information.
//!
//! The strategy is to decode the image into raw pixels and re-encode
//! it, which naturally strips all metadata segments. This is the
//! simplest approach that guarantees completeness.

use image::ImageFormat;

use crate::MediaError;

/// Strip all EXIF/GPS/IPTC/XMP metadata from an image.
///
/// This works by decoding the image to raw pixels and re-encoding it
/// in the same format. The re-encoded output contains only pixel data
/// and essential format headers — no metadata.
///
/// # Errors
///
/// Returns [`MediaError`] if the image cannot be decoded or re-encoded.
pub fn strip_exif_data(data: &[u8], format: ImageFormat) -> Result<Vec<u8>, MediaError> {
    let img = image::load_from_memory(data)
        .map_err(|e| MediaError::Processing(format!("failed to decode for EXIF strip: {e}")))?;

    let mut output = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut output);

    img.write_to(&mut cursor, format).map_err(|e| {
        MediaError::Processing(format!("failed to re-encode after EXIF strip: {e}"))
    })?;

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal valid PNG (1x1 red pixel) for testing.
    fn minimal_png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]));
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn strip_roundtrip_png() {
        let original = minimal_png();
        let stripped = strip_exif_data(&original, ImageFormat::Png).unwrap();
        // The stripped output should still be a valid PNG.
        let decoded = image::load_from_memory(&stripped).unwrap();
        assert_eq!(decoded.width(), 1);
        assert_eq!(decoded.height(), 1);
    }

    #[test]
    fn strip_invalid_data_fails() {
        let garbage = vec![0x00, 0x01, 0x02, 0x03, 0x04];
        let result = strip_exif_data(&garbage, ImageFormat::Png);
        assert!(result.is_err());
    }
}
