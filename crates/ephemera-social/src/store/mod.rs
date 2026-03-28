//! Concrete SQLite-backed implementations of the social service traits.
//!
//! Provides [`SqliteSocialServices`] which implements [`ConnectionService`],
//! [`FeedService`], and [`BlockService`] using the metadata database from
//! `ephemera-store`.

use std::sync::Mutex;

use ephemera_store::MetadataDb;
use ephemera_types::{IdentityKey, Timestamp};

use crate::block::BlockService;
use crate::connection::{
    Connection, ConnectionError, ConnectionService, ConnectionStatus, MAX_CONNECTION_MESSAGE_LEN,
};
use crate::feed::{FeedCursor, FeedItem, FeedPage, FeedService};
use crate::SocialError;

/// Concrete service container backed by a shared SQLite metadata database.
///
/// All methods lock the database mutex for the duration of their query.
/// This is acceptable for single-node use; networked deployments should
/// use a connection pool or sharded approach.
pub struct SqliteSocialServices {
    db: Mutex<MetadataDb>,
}

impl SqliteSocialServices {
    /// Create a new service container wrapping the given database.
    pub fn new(db: MetadataDb) -> Self {
        Self { db: Mutex::new(db) }
    }

    /// Internal helper: convert a `ConnectionStatus` to the string stored in SQLite.
    fn status_to_str(status: ConnectionStatus) -> &'static str {
        match status {
            ConnectionStatus::PendingOutgoing => "pending_outgoing",
            ConnectionStatus::PendingIncoming => "pending_incoming",
            ConnectionStatus::Active => "connected",
            ConnectionStatus::Blocked => "blocked",
        }
    }

    /// Internal helper: parse a status string from SQLite.
    fn str_to_status(s: &str) -> ConnectionStatus {
        match s {
            "pending_outgoing" => ConnectionStatus::PendingOutgoing,
            "pending_incoming" => ConnectionStatus::PendingIncoming,
            "connected" => ConnectionStatus::Active,
            "blocked" => ConnectionStatus::Blocked,
            _ => ConnectionStatus::PendingOutgoing, // fallback
        }
    }
}

mod block;
mod connection;
mod feed;
mod feed_discover;
mod group;
mod group_chat;
mod mention;
mod reaction;
mod topic_room;

pub use connection::receive_connection_request;
pub use feed_discover::discover_feed;

fn bytes_to_content_hash(bytes: &[u8]) -> ephemera_types::ContentId {
    if bytes.len() == 33 {
        ephemera_types::ContentId::from_wire_bytes(bytes)
            .unwrap_or_else(|| ephemera_types::ContentId::from_digest([0u8; 32]))
    } else if bytes.len() == 32 {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        ephemera_types::ContentId::from_digest(arr)
    } else {
        ephemera_types::ContentId::from_digest([0u8; 32])
    }
}

fn bytes_to_identity_key(bytes: &[u8]) -> IdentityKey {
    let mut arr = [0u8; 32];
    if bytes.len() == 32 {
        arr.copy_from_slice(bytes);
    }
    IdentityKey::from_bytes(arr)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
