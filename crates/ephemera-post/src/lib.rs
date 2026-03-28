//! Post data model and validation for the Ephemera platform.
//!
//! This crate defines the [`Post`] struct, its content variants, a fluent
//! [`PostBuilder`], and validation logic for incoming posts.

pub mod builder;
pub mod canonical;
pub mod content;
pub mod post;
pub mod validation;

pub use builder::PostBuilder;
pub use canonical::canonical_bytes;
pub use content::PostContent;
pub use post::{Post, PowProof};
pub use validation::validate_post;

/// Errors from post creation and validation.
#[derive(Debug, thiserror::Error)]
pub enum PostError {
    /// An error during post construction.
    #[error("build error: {0}")]
    Build(String),

    /// An error during post validation.
    #[error("validation error: {0}")]
    Validation(String),
}
