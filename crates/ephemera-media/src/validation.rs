//! Media file validation.
//!
//! Validates uploaded media against protocol constraints before processing:
//! file type, file size, and image dimensions.

use crate::MediaError;

/// Maximum input file size in bytes (10 MB).
pub const MAX_INPUT_SIZE: usize = 10 * 1024 * 1024;

/// Maximum output file size in bytes after processing (5 MiB).
pub const MAX_OUTPUT_SIZE: usize = 5 * 1024 * 1024;

/// Maximum image width in pixels.
pub const MAX_WIDTH: u32 = 1920;

/// Maximum image height in pixels.
pub const MAX_HEIGHT: u32 = 1080;

/// Supported media types for the PoC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedFormat {
    /// JPEG image.
    Jpeg,
    /// PNG image.
    Png,
    /// WebP image.
    WebP,
}

impl std::fmt::Display for SupportedFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jpeg => write!(f, "JPEG"),
            Self::Png => write!(f, "PNG"),
            Self::WebP => write!(f, "WebP"),
        }
    }
}

/// Detect the image format from the first bytes of data.
///
/// Returns the detected format or an error if the file type is not
/// supported.
pub fn detect_format(data: &[u8]) -> Result<SupportedFormat, MediaError> {
    if data.len() < 4 {
        return Err(MediaError::UnsupportedFormat(
            "file too small to identify".into(),
        ));
    }

    // JPEG: starts with FF D8 FF
    if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Ok(SupportedFormat::Jpeg);
    }

    // PNG: starts with 89 50 4E 47
    if data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47 {
        return Ok(SupportedFormat::Png);
    }

    // WebP: starts with RIFF....WEBP
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Ok(SupportedFormat::WebP);
    }

    Err(MediaError::UnsupportedFormat(
        "only JPEG, PNG, and WebP are supported".into(),
    ))
}

/// Validate raw media bytes against size and format constraints.
///
/// # Errors
///
/// Returns [`MediaError`] if the file is too large, too small, or an
/// unsupported format.
pub fn validate_media(data: &[u8]) -> Result<SupportedFormat, MediaError> {
    if data.is_empty() {
        return Err(MediaError::Validation("empty file".into()));
    }

    if data.len() > MAX_INPUT_SIZE {
        return Err(MediaError::Validation(format!(
            "file size {} bytes exceeds maximum {} bytes",
            data.len(),
            MAX_INPUT_SIZE
        )));
    }

    let format = detect_format(data)?;

    // Try to decode to check dimensions.
    let img = image::load_from_memory(data)
        .map_err(|e| MediaError::Validation(format!("failed to decode image: {e}")))?;

    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err(MediaError::Validation("image has zero dimensions".into()));
    }

    Ok(format)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_jpeg() {
        let header = [0xFF, 0xD8, 0xFF, 0xE0, 0x00];
        assert_eq!(detect_format(&header).unwrap(), SupportedFormat::Jpeg);
    }

    #[test]
    fn detect_png() {
        let header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(detect_format(&header).unwrap(), SupportedFormat::Png);
    }

    #[test]
    fn detect_webp() {
        let mut header = vec![0u8; 12];
        header[0..4].copy_from_slice(b"RIFF");
        header[8..12].copy_from_slice(b"WEBP");
        assert_eq!(detect_format(&header).unwrap(), SupportedFormat::WebP);
    }

    #[test]
    fn reject_unknown_format() {
        let garbage = [0x00, 0x01, 0x02, 0x03];
        assert!(detect_format(&garbage).is_err());
    }

    #[test]
    fn reject_empty() {
        assert!(validate_media(&[]).is_err());
    }

    #[test]
    fn reject_oversized() {
        let big = vec![0xFF; MAX_INPUT_SIZE + 1];
        assert!(validate_media(&big).is_err());
    }

    #[test]
    fn reject_too_small_to_identify() {
        assert!(detect_format(&[0x00]).is_err());
    }
}
