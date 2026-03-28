//! Storage engine for the Ephemera decentralized social platform.
//!
//! This crate provides two storage layers:
//!
//! - **Content store** ([`ContentStore`]): content-addressed blob storage on the
//!   filesystem, using BLAKE3 hashes as paths (first 2 hex chars as directory,
//!   remainder as filename).
//! - **Metadata store** ([`MetadataDb`]): SQLite database for relational metadata
//!   (posts, connections, messages, profiles, blocks).
//!
//! A background [`GarbageCollector`] periodically scans for expired content
//! and removes both blobs and metadata rows.

pub mod content_store;
pub mod gc;
pub mod media_store;
pub mod metadata;
pub mod migrations;
pub mod query;

pub use content_store::ContentStore;
pub use gc::{ClockFn, GarbageCollector, GcConfig, GcReport};
pub use media_store::{
    get_media_attachment, get_media_chunk, get_thumbnail_hash, insert_media_attachment,
    insert_media_chunk, list_chunks_for_media, list_media_for_post, MediaAttachment, StoredChunk,
};
pub use metadata::MetadataDb;
pub use query::QueryEngine;

/// Errors from storage operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// An I/O error from the filesystem content store.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// An error from the SQLite metadata store.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// The requested item was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// A data integrity violation (e.g., hash mismatch).
    #[error("integrity error: {0}")]
    Integrity(String),
}
