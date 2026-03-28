//! Video metadata stripping.
//!
//! Removes privacy-sensitive metadata from MP4 containers: creation
//! timestamps, GPS coordinates, camera/device info, encoder strings,
//! and any other atoms that could deanonymize the user.
//!
//! Strategy: rebuild the MP4 container keeping only essential structural
//! atoms (ftyp, moov track structure, mdat). All user-data atoms (udta),
//! metadata atoms (meta), and non-essential fields are stripped.
//!
//! NOTE: For v1, this operates on the raw MP4 bytes using a conservative
//! approach -- we zero out known metadata fields and strip udta/meta boxes.
//! Full re-muxing would require FFmpeg; this provides defense-in-depth for
//! pre-encoded H.264 input.

use crate::MediaError;

/// Known MP4 box types that carry user metadata and should be removed.
const METADATA_BOX_TYPES: &[&[u8; 4]] = &[
    b"udta", // User data box (contains GPS, copyright, etc.)
    b"meta", // Metadata box (iTunes-style, XMP, etc.)
    b"uuid", // UUID extension boxes (often proprietary metadata)
];

/// Strip metadata from an MP4 file.
///
/// This scans the top-level and moov-level boxes in the MP4 and zeros out
/// any boxes that contain user metadata (udta, meta, uuid). It also zeros
/// the creation/modification timestamps in the mvhd and tkhd boxes.
///
/// The resulting bytes maintain the same length (box sizes are preserved,
/// contents are zeroed), keeping the file structurally valid.
///
/// # Errors
///
/// Returns [`MediaError`] if the MP4 structure cannot be parsed.
pub fn strip_video_metadata(data: &[u8]) -> Result<Vec<u8>, MediaError> {
    let mut output = data.to_vec();
    let len = output.len();
    strip_boxes_recursive(&mut output, 0, len)?;
    Ok(output)
}

/// Recursively scan and strip metadata boxes within a byte range.
fn strip_boxes_recursive(data: &mut [u8], start: usize, end: usize) -> Result<(), MediaError> {
    let mut pos = start;

    while pos + 8 <= end {
        let box_size = read_box_size(data, pos)?;
        let box_type = &data[pos + 4..pos + 8];

        // Determine actual box size for navigation.
        let actual_size = if box_size == 0 {
            // Box extends to end of file.
            end - pos
        } else if box_size == 1 {
            // 64-bit extended size.
            if pos + 16 > end {
                return Err(MediaError::Processing(
                    "truncated MP4 box with extended size".into(),
                ));
            }
            read_u64(data, pos + 8) as usize
        } else {
            box_size as usize
        };

        if actual_size < 8 || pos + actual_size > end {
            // Invalid box size -- stop scanning this level.
            break;
        }

        // Check if this is a metadata box to strip.
        if is_metadata_box(box_type) {
            zero_box_contents(data, pos, actual_size);
        } else if box_type == b"moov" || box_type == b"trak" || box_type == b"mdia" {
            // Recurse into container boxes.
            let header_size = if box_size == 1 { 16 } else { 8 };
            strip_boxes_recursive(data, pos + header_size, pos + actual_size)?;
        } else if box_type == b"mvhd" {
            strip_mvhd_timestamps(data, pos, actual_size);
        } else if box_type == b"tkhd" {
            strip_tkhd_timestamps(data, pos, actual_size);
        }

        pos += actual_size;
    }

    Ok(())
}

/// Read a 32-bit big-endian box size at the given position.
fn read_box_size(data: &[u8], pos: usize) -> Result<u32, MediaError> {
    if pos + 4 > data.len() {
        return Err(MediaError::Processing("truncated MP4 box header".into()));
    }
    Ok(u32::from_be_bytes([
        data[pos],
        data[pos + 1],
        data[pos + 2],
        data[pos + 3],
    ]))
}

/// Read a 64-bit big-endian value at the given position.
fn read_u64(data: &[u8], pos: usize) -> u64 {
    u64::from_be_bytes([
        data[pos],
        data[pos + 1],
        data[pos + 2],
        data[pos + 3],
        data[pos + 4],
        data[pos + 5],
        data[pos + 6],
        data[pos + 7],
    ])
}

/// Check if a 4-byte box type is a known metadata box.
fn is_metadata_box(box_type: &[u8]) -> bool {
    METADATA_BOX_TYPES
        .iter()
        .any(|mt| box_type == mt.as_slice())
}

/// Zero out the contents of a box (preserve size and type header).
fn zero_box_contents(data: &mut [u8], pos: usize, size: usize) {
    // Keep the 8-byte header (size + type), zero everything else.
    let content_start = pos + 8;
    let content_end = pos + size;
    if content_start < content_end && content_end <= data.len() {
        data[content_start..content_end].fill(0);
    }
}

/// Zero the creation and modification timestamps in an mvhd box.
///
/// mvhd layout (version 0): 8-byte header, 1-byte version, 3-byte flags,
/// then 4-byte creation_time, 4-byte modification_time.
/// mvhd layout (version 1): 8-byte header, 1-byte version, 3-byte flags,
/// then 8-byte creation_time, 8-byte modification_time.
fn strip_mvhd_timestamps(data: &mut [u8], pos: usize, size: usize) {
    if size < 20 {
        return;
    }
    let header = 8; // box header size
    let version = data[pos + header];

    if version == 0 && size >= header + 4 + 8 {
        // Zero creation_time (4 bytes at offset 4) and modification_time (4 bytes at offset 8).
        let ts_start = pos + header + 4;
        data[ts_start..ts_start + 8].fill(0);
    } else if version == 1 && size >= header + 4 + 16 {
        // Zero creation_time (8 bytes at offset 4) and modification_time (8 bytes at offset 12).
        let ts_start = pos + header + 4;
        data[ts_start..ts_start + 16].fill(0);
    }
}

/// Zero the creation and modification timestamps in a tkhd box.
///
/// Same layout pattern as mvhd.
fn strip_tkhd_timestamps(data: &mut [u8], pos: usize, size: usize) {
    // tkhd has the same timestamp layout as mvhd for the first fields.
    strip_mvhd_timestamps(data, pos, size);
}

/// Verify that an MP4's udta and meta boxes have been stripped.
///
/// Returns `true` if no active metadata boxes are found at the top level
/// or inside the moov box.
pub fn verify_metadata_stripped(data: &[u8]) -> bool {
    !scan_for_metadata_boxes(data, 0, data.len())
}

/// Scan for active (non-zeroed) metadata boxes in a byte range.
fn scan_for_metadata_boxes(data: &[u8], start: usize, end: usize) -> bool {
    let mut pos = start;

    while pos + 8 <= end {
        let box_size = match read_box_size(data, pos) {
            Ok(s) => s,
            Err(_) => break,
        };
        let box_type = &data[pos + 4..pos + 8];

        let actual_size = if box_size == 0 {
            end - pos
        } else if box_size == 1 {
            if pos + 16 > end {
                break;
            }
            read_u64(data, pos + 8) as usize
        } else {
            box_size as usize
        };

        if actual_size < 8 || pos + actual_size > end {
            break;
        }

        if is_metadata_box(box_type) {
            // Check if content is non-zero (active metadata).
            let content_start = pos + 8;
            let content_end = pos + actual_size;
            if content_start < content_end {
                let has_content = data[content_start..content_end].iter().any(|&b| b != 0);
                if has_content {
                    return true;
                }
            }
        } else if box_type == b"moov" || box_type == b"trak" || box_type == b"mdia" {
            let header_size = if box_size == 1 { 16 } else { 8 };
            if scan_for_metadata_boxes(data, pos + header_size, pos + actual_size) {
                return true;
            }
        }

        pos += actual_size;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal MP4-like structure with a udta box inside moov.
    fn mp4_with_udta() -> Vec<u8> {
        let mut data = Vec::new();

        // ftyp box (20 bytes).
        let ftyp_size = 20u32;
        data.extend_from_slice(&ftyp_size.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(b"isom");
        data.extend_from_slice(&[0u8; 8]);

        // moov box containing a udta box.
        let udta_content = b"GPS data here!!!"; // 16 bytes of fake metadata
        let udta_size = 8u32 + udta_content.len() as u32;

        let moov_size = 8u32 + udta_size;
        data.extend_from_slice(&moov_size.to_be_bytes());
        data.extend_from_slice(b"moov");

        data.extend_from_slice(&udta_size.to_be_bytes());
        data.extend_from_slice(b"udta");
        data.extend_from_slice(udta_content);

        data
    }

    #[test]
    fn strip_removes_udta_content() {
        let original = mp4_with_udta();
        assert!(scan_for_metadata_boxes(&original, 0, original.len()));

        let stripped = strip_video_metadata(&original).unwrap();
        assert_eq!(stripped.len(), original.len());
        assert!(!scan_for_metadata_boxes(&stripped, 0, stripped.len()));
        assert!(verify_metadata_stripped(&stripped));
    }

    #[test]
    fn verify_clean_file() {
        // A minimal MP4 with no metadata boxes.
        let mut data = Vec::new();
        let ftyp_size = 20u32;
        data.extend_from_slice(&ftyp_size.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(b"isom");
        data.extend_from_slice(&[0u8; 8]);

        let moov_size = 8u32;
        data.extend_from_slice(&moov_size.to_be_bytes());
        data.extend_from_slice(b"moov");

        assert!(verify_metadata_stripped(&data));
    }
}
