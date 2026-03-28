//! Error types for the moderation subsystem.

/// Errors arising from moderation operations.
#[derive(Debug, thiserror::Error)]
pub enum ModerationError {
    /// The report reason description was too long.
    #[error("report description exceeds {max} characters (got {got})")]
    DescriptionTooLong {
        /// Actual character count.
        got: usize,
        /// Maximum allowed.
        max: usize,
    },

    /// A duplicate report already exists.
    #[error("duplicate report for content {content_id}")]
    DuplicateReport {
        /// The content hash that was already reported.
        content_id: String,
    },

    /// The blocklist file could not be loaded or saved.
    #[error("blocklist I/O error: {reason}")]
    BlocklistIo {
        /// Human-readable reason.
        reason: String,
    },
}
