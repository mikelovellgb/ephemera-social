//! Video-specific validation.
//!
//! Validates MP4 files against Ephemera protocol constraints:
//! format (H.264/MP4), duration (max 3 minutes), file size (max 50 MiB),
//! and codec requirements.

use std::io::Cursor;

use crate::MediaError;

/// Maximum video input file size: 50 MiB.
pub const MAX_VIDEO_INPUT_SIZE: usize = 50 * 1024 * 1024;

/// Maximum video duration in milliseconds: 3 minutes.
pub const MAX_VIDEO_DURATION_MS: u64 = 3 * 60 * 1000;

/// Minimum file size to be a valid MP4 (needs at least ftyp box header).
const MIN_MP4_SIZE: usize = 8;

/// Validated video metadata extracted from an MP4 file.
#[derive(Debug, Clone)]
pub struct VideoInfo {
    /// Video duration in milliseconds.
    pub duration_ms: u64,
    /// Video width in pixels.
    pub width: u32,
    /// Video height in pixels.
    pub height: u32,
    /// Video codec identifier (e.g. "avc1" for H.264).
    pub codec: String,
    /// File size in bytes.
    pub file_size: usize,
    /// Timescale of the video track.
    pub timescale: u32,
}

/// Detect whether raw bytes start with an MP4 `ftyp` box.
///
/// MP4 files begin with a box whose type field (bytes 4..8) is `ftyp`.
/// The first 4 bytes are the box size (big-endian u32).
pub fn is_mp4(data: &[u8]) -> bool {
    if data.len() < MIN_MP4_SIZE {
        return false;
    }
    &data[4..8] == b"ftyp"
}

/// Validate raw bytes as a valid MP4 video and extract metadata.
///
/// Checks:
/// 1. MP4 magic bytes (`ftyp` box)
/// 2. File size <= 50 MiB
/// 3. Parseable MP4 container with at least one video track
/// 4. H.264 (avc1/avc3) codec
/// 5. Duration <= 3 minutes
///
/// # Errors
///
/// Returns [`MediaError`] if any check fails.
pub fn validate_video(data: &[u8]) -> Result<VideoInfo, MediaError> {
    // Check minimum size.
    if data.len() < MIN_MP4_SIZE {
        return Err(MediaError::Validation(
            "file too small to be a valid MP4".into(),
        ));
    }

    // Check ftyp magic bytes.
    if !is_mp4(data) {
        return Err(MediaError::Validation(
            "not an MP4 file (missing ftyp box)".into(),
        ));
    }

    // Check file size limit.
    if data.len() > MAX_VIDEO_INPUT_SIZE {
        return Err(MediaError::Validation(format!(
            "video file size {} bytes exceeds maximum {} bytes",
            data.len(),
            MAX_VIDEO_INPUT_SIZE,
        )));
    }

    // Parse the MP4 container.
    let cursor = Cursor::new(data);
    let reader = mp4::Mp4Reader::read_header(cursor, data.len() as u64)
        .map_err(|e| MediaError::Validation(format!("failed to parse MP4 container: {e}")))?;

    // Find the first video track.
    let video_track = find_video_track(&reader)?;

    // Validate codec is H.264.
    let media_type = video_track
        .media_type()
        .map_err(|e| MediaError::Validation(format!("cannot determine video codec: {e}")))?;
    if media_type != mp4::MediaType::H264 {
        return Err(MediaError::Validation(format!(
            "unsupported video codec '{media_type}'; only H.264 is supported"
        )));
    }
    let codec = media_type.to_string();

    // Extract dimensions.
    let width = u32::from(video_track.width());
    let height = u32::from(video_track.height());
    if width == 0 || height == 0 {
        return Err(MediaError::Validation("video has zero dimensions".into()));
    }

    // Extract duration.
    let duration = video_track.duration();
    let duration_ms = duration.as_millis() as u64;

    // Check duration limit.
    if duration_ms > MAX_VIDEO_DURATION_MS {
        return Err(MediaError::Validation(format!(
            "video duration {duration_ms}ms exceeds maximum {MAX_VIDEO_DURATION_MS}ms (3 minutes)"
        )));
    }

    Ok(VideoInfo {
        duration_ms,
        width,
        height,
        codec,
        file_size: data.len(),
        timescale: video_track.timescale(),
    })
}

/// Find the first video track in an MP4 reader.
fn find_video_track<'a>(
    reader: &'a mp4::Mp4Reader<Cursor<&'a [u8]>>,
) -> Result<&'a mp4::Mp4Track, MediaError> {
    for track_id in 1..=reader.tracks().len() as u32 {
        if let Some(track) = reader.tracks().get(&track_id) {
            if track.track_type().ok() == Some(mp4::TrackType::Video) {
                return Ok(track);
            }
        }
    }
    Err(MediaError::Validation(
        "MP4 file contains no video track".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_mp4_detects_ftyp() {
        let mut data = vec![0u8; 32];
        // Box size = 32, box type = ftyp
        data[0..4].copy_from_slice(&32u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        assert!(is_mp4(&data));
    }

    #[test]
    fn is_mp4_rejects_non_mp4() {
        assert!(!is_mp4(&[0x89, 0x50, 0x4E, 0x47, 0x00, 0x00, 0x00, 0x00]));
    }

    #[test]
    fn is_mp4_rejects_short_data() {
        assert!(!is_mp4(&[0x00, 0x01]));
    }

    #[test]
    fn validate_video_rejects_empty() {
        assert!(validate_video(&[]).is_err());
    }

    #[test]
    fn validate_video_rejects_non_mp4() {
        let png_header = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];
        let result = validate_video(&png_header);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ftyp"));
    }

    #[test]
    fn validate_video_rejects_oversized() {
        let mut data = vec![0u8; MAX_VIDEO_INPUT_SIZE + 1];
        // Set ftyp header so it passes the magic byte check before size check.
        data[0..4].copy_from_slice(&8u32.to_be_bytes());
        data[4..8].copy_from_slice(b"ftyp");
        let result = validate_video(&data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum"));
    }
}
