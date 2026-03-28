//! Media processing pipeline for the Ephemera platform.
//!
//! Handles image and video validation, metadata stripping, resizing,
//! chunking, and content hashing. Supports JPEG, PNG, and WebP images,
//! plus H.264/MP4 video (v1).

pub mod chunker;
pub mod exif;
pub mod pipeline;
pub mod resize;
pub mod validation;
pub mod video;
pub mod video_metadata;
pub mod video_validation;

#[cfg(test)]
mod integration_tests;
/// Minimal MP4 test file builder, exposed for integration tests.
pub mod test_mp4;

pub use chunker::{Chunk, ChunkInfo, ChunkManifest, ContentChunker};
pub use pipeline::{MediaPipeline, ProcessedContent, ProcessedMedia};
pub use resize::ResizeResult;
pub use validation::{validate_media, SupportedFormat};
pub use video::{ProcessedVideo, VideoProcessor};
pub use video_validation::VideoInfo;

/// Errors from media processing.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// The media file failed validation (wrong format, too large, etc.).
    #[error("validation error: {0}")]
    Validation(String),

    /// The media format is not supported.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),

    /// An error occurred during processing (decode, resize, encode).
    #[error("processing error: {0}")]
    Processing(String),
}
