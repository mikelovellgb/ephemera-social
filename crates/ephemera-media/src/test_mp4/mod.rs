//! Minimal MP4 test file builder.
//!
//! Constructs valid MP4 files programmatically for use in tests.
//! Produces the minimum viable MP4 structure: ftyp + moov + mdat,
//! with a single H.264 video track.
//!
//! This is test-only code and not part of the public API.

mod atoms;
use atoms::{build_ftyp, build_mdat, build_moov};

/// Build a minimal valid MP4 file with a single H.264 video track.
///
/// Parameters:
/// - `width`: video width in pixels
/// - `height`: video height in pixels
/// - `duration_ms`: video duration in milliseconds
///
/// The returned bytes form a structurally valid MP4 that the `mp4` crate
/// can parse, containing proper ftyp, moov (mvhd + trak), and mdat boxes.
pub fn build_test_mp4(width: u16, height: u16, duration_ms: u64) -> Vec<u8> {
    let timescale: u32 = 1000;
    let duration: u32 = duration_ms as u32;
    let sample_count: u32 = duration_ms.div_ceil(33).max(1) as u32;

    let ftyp = build_ftyp();
    let moov = build_moov(width, height, timescale, duration, sample_count);
    let mdat = build_mdat(sample_count);

    let mut mp4 = Vec::new();
    mp4.extend_from_slice(&ftyp);
    mp4.extend_from_slice(&moov);
    mp4.extend_from_slice(&mdat);
    mp4
}

/// Build a test MP4 that is oversized for rejection tests.
///
/// Creates a valid header but pads with a large mdat box to exceed
/// the given `target_size`.
#[allow(dead_code)]
pub fn build_oversized_mp4(target_size: usize) -> Vec<u8> {
    let ftyp = build_ftyp();
    let moov = build_moov(640, 480, 1000, 1000, 30);

    let header_size = ftyp.len() + moov.len() + 8;
    let mdat_content_size = target_size.saturating_sub(header_size);
    let mdat_size = (mdat_content_size + 8) as u32;

    let mut mp4 = Vec::new();
    mp4.extend_from_slice(&ftyp);
    mp4.extend_from_slice(&moov);
    mp4.extend_from_slice(&mdat_size.to_be_bytes());
    mp4.extend_from_slice(b"mdat");
    mp4.resize(mp4.len() + mdat_content_size, 0x00);
    mp4
}

/// Build a test MP4 with a specific duration for duration rejection tests.
pub fn build_long_duration_mp4(duration_ms: u64) -> Vec<u8> {
    build_test_mp4(640, 480, duration_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_test_mp4_is_valid() {
        let data = build_test_mp4(640, 480, 1000);
        assert_eq!(&data[4..8], b"ftyp");
        assert!(data.len() > 100);
    }

    #[test]
    fn build_test_mp4_various_sizes() {
        let short = build_test_mp4(320, 240, 100);
        assert_eq!(&short[4..8], b"ftyp");

        let long = build_test_mp4(1920, 1080, 180_000);
        assert_eq!(&long[4..8], b"ftyp");
    }
}
