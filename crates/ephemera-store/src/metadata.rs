//! SQLite-backed metadata store.
//!
//! Stores relational metadata for posts, connections, messages, profiles,
//! and blocks. The actual content blobs live in [`super::ContentStore`];
//! this database holds only metadata and indexes needed for queries.

use crate::migrations;
use crate::StoreError;
use rusqlite::Connection;
use std::path::Path;

/// SQLite metadata database.
///
/// Wraps a single [`rusqlite::Connection`] configured with WAL mode and
/// pragmas tuned for Ephemera's workload.
pub struct MetadataDb {
    conn: Connection,
}

impl MetadataDb {
    /// Open (or create) a metadata database at `path`.
    ///
    /// Applies all pending migrations on open and configures SQLite
    /// pragmas for performance.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::configure_pragmas(&conn)?;
        let mut db = Self { conn };
        migrations::run_migrations(&mut db)?;
        Ok(db)
    }

    /// Open an in-memory database (useful for tests).
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_pragmas(&conn)?;
        let mut db = Self { conn };
        migrations::run_migrations(&mut db)?;
        Ok(db)
    }

    /// Return a reference to the underlying connection (for queries).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Return a mutable reference to the underlying connection.
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Apply recommended SQLite pragmas for Ephemera.
    fn configure_pragmas(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -8000;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;
             PRAGMA auto_vacuum = INCREMENTAL;
             PRAGMA temp_store = MEMORY;",
        )?;
        Ok(())
    }
}

/// Full DDL for the initial schema (version 1).
///
/// Called by the migration system to bootstrap a fresh database.
pub(crate) fn schema_v1() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS schema_version (
        version     INTEGER NOT NULL,
        applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
        description TEXT
    );

    CREATE TABLE IF NOT EXISTS posts (
        content_hash    BLOB NOT NULL PRIMARY KEY,
        author_pubkey   BLOB NOT NULL,
        sequence_number INTEGER NOT NULL,
        created_at      INTEGER NOT NULL,
        expires_at      INTEGER NOT NULL,
        ttl_seconds     INTEGER NOT NULL,
        parent_hash     BLOB,
        root_hash       BLOB,
        depth           INTEGER NOT NULL DEFAULT 0,
        body_preview    TEXT,
        media_count     INTEGER NOT NULL DEFAULT 0,
        has_media       INTEGER NOT NULL DEFAULT 0,
        language_hint   TEXT,
        pow_difficulty  INTEGER NOT NULL DEFAULT 0,
        identity_age    INTEGER NOT NULL DEFAULT 0,
        is_tombstone    INTEGER NOT NULL DEFAULT 0,
        tombstone_at    INTEGER,
        received_at     INTEGER NOT NULL,
        epoch_number    INTEGER NOT NULL,
        signature       BLOB NOT NULL,
        blob_hash       TEXT
    );

    CREATE INDEX IF NOT EXISTS idx_posts_author
        ON posts(author_pubkey, created_at DESC);
    CREATE INDEX IF NOT EXISTS idx_posts_expires
        ON posts(expires_at) WHERE is_tombstone = 0;
    CREATE INDEX IF NOT EXISTS idx_posts_parent
        ON posts(parent_hash) WHERE parent_hash IS NOT NULL;
    CREATE INDEX IF NOT EXISTS idx_posts_root
        ON posts(root_hash) WHERE root_hash IS NOT NULL;
    CREATE INDEX IF NOT EXISTS idx_posts_created
        ON posts(created_at DESC);
    CREATE INDEX IF NOT EXISTS idx_posts_epoch
        ON posts(epoch_number);
    CREATE INDEX IF NOT EXISTS idx_posts_tombstone
        ON posts(is_tombstone, tombstone_at) WHERE is_tombstone = 1;

    CREATE TABLE IF NOT EXISTS post_tags (
        content_hash  BLOB NOT NULL REFERENCES posts(content_hash) ON DELETE CASCADE,
        tag           TEXT NOT NULL,
        PRIMARY KEY (content_hash, tag)
    );
    CREATE INDEX IF NOT EXISTS idx_tags_lookup ON post_tags(tag, content_hash);

    CREATE TABLE IF NOT EXISTS post_mentions (
        content_hash   BLOB NOT NULL REFERENCES posts(content_hash) ON DELETE CASCADE,
        mentioned_key  BLOB NOT NULL,
        display_hint   TEXT,
        byte_start     INTEGER NOT NULL,
        byte_end       INTEGER NOT NULL,
        PRIMARY KEY (content_hash, mentioned_key)
    );
    CREATE INDEX IF NOT EXISTS idx_mentions_key
        ON post_mentions(mentioned_key, content_hash);

    CREATE TABLE IF NOT EXISTS connections (
        local_pubkey    BLOB NOT NULL,
        remote_pubkey   BLOB NOT NULL,
        status          TEXT NOT NULL,
        created_at      INTEGER NOT NULL,
        updated_at      INTEGER NOT NULL,
        display_name    TEXT,
        PRIMARY KEY (local_pubkey, remote_pubkey)
    );
    CREATE INDEX IF NOT EXISTS idx_connections_status
        ON connections(local_pubkey, status);

    CREATE TABLE IF NOT EXISTS follows (
        follower_pubkey  BLOB NOT NULL,
        followed_pubkey  BLOB NOT NULL,
        created_at       INTEGER NOT NULL,
        PRIMARY KEY (follower_pubkey, followed_pubkey)
    );
    CREATE INDEX IF NOT EXISTS idx_follows_followed
        ON follows(followed_pubkey, created_at DESC);

    CREATE TABLE IF NOT EXISTS blocks (
        blocker_pubkey  BLOB NOT NULL,
        blocked_pubkey  BLOB NOT NULL,
        created_at      INTEGER NOT NULL,
        reason          TEXT,
        PRIMARY KEY (blocker_pubkey, blocked_pubkey)
    );

    CREATE TABLE IF NOT EXISTS mutes (
        muter_pubkey    BLOB NOT NULL,
        muted_pubkey    BLOB NOT NULL,
        created_at      INTEGER NOT NULL,
        expires_at      INTEGER,
        PRIMARY KEY (muter_pubkey, muted_pubkey)
    );

    CREATE TABLE IF NOT EXISTS profiles (
        pubkey         BLOB NOT NULL PRIMARY KEY,
        display_name   TEXT,
        bio            TEXT,
        avatar_cid     BLOB,
        updated_at     INTEGER NOT NULL,
        signature      BLOB NOT NULL,
        received_at    INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS conversations (
        conversation_id  BLOB NOT NULL PRIMARY KEY,
        our_pubkey       BLOB NOT NULL,
        their_pubkey     BLOB NOT NULL,
        last_message_at  INTEGER,
        unread_count     INTEGER NOT NULL DEFAULT 0,
        is_request       INTEGER NOT NULL DEFAULT 0,
        created_at       INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_conversations_last
        ON conversations(last_message_at DESC);

    CREATE TABLE IF NOT EXISTS messages (
        message_id       BLOB NOT NULL PRIMARY KEY,
        conversation_id  BLOB NOT NULL REFERENCES conversations(conversation_id),
        sender_pubkey    BLOB NOT NULL,
        received_at      INTEGER NOT NULL,
        expires_at       INTEGER NOT NULL,
        is_read          INTEGER NOT NULL DEFAULT 0,
        body_preview     TEXT,
        has_media        INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_messages_conversation
        ON messages(conversation_id, received_at DESC);
    CREATE INDEX IF NOT EXISTS idx_messages_expires
        ON messages(expires_at);

    CREATE TABLE IF NOT EXISTS epoch_keys (
        epoch_number   INTEGER NOT NULL PRIMARY KEY,
        created_at     INTEGER NOT NULL,
        expires_at     INTEGER NOT NULL,
        is_deleted     INTEGER NOT NULL DEFAULT 0,
        deleted_at     INTEGER,
        content_count  INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_epoch_expires
        ON epoch_keys(expires_at) WHERE is_deleted = 0;

    CREATE TABLE IF NOT EXISTS local_state (
        key    TEXT NOT NULL PRIMARY KEY,
        value  TEXT NOT NULL
    );
    "#
}

/// Migration v2: add `message` and `initiator_pubkey` columns to connections.
pub(crate) fn schema_v2() -> &'static str {
    r#"
    ALTER TABLE connections ADD COLUMN message TEXT;
    ALTER TABLE connections ADD COLUMN initiator_pubkey BLOB;
    "#
}

/// Migration v3: add `message_requests` table for stranger DM request handling.
pub(crate) fn schema_v3() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS message_requests (
        sender_pubkey    BLOB NOT NULL,
        recipient_pubkey BLOB NOT NULL,
        status           TEXT NOT NULL DEFAULT 'pending',
        created_at       INTEGER NOT NULL,
        PRIMARY KEY (sender_pubkey, recipient_pubkey)
    );
    CREATE INDEX IF NOT EXISTS idx_message_requests_recipient
        ON message_requests(recipient_pubkey, status);
    "#
}

/// Migration v4: add ratchet_sessions table for Double Ratchet state persistence.
pub(crate) fn schema_v4() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS ratchet_sessions (
        conversation_id     TEXT NOT NULL PRIMARY KEY,
        peer_pubkey         BLOB NOT NULL,
        root_key            BLOB NOT NULL,
        send_chain_key      BLOB NOT NULL,
        recv_chain_key      BLOB NOT NULL,
        send_ratchet_secret BLOB NOT NULL,
        send_ratchet_pubkey BLOB NOT NULL,
        recv_ratchet_pubkey BLOB,
        send_count          INTEGER NOT NULL DEFAULT 0,
        recv_count          INTEGER NOT NULL DEFAULT 0,
        prev_send_count     INTEGER NOT NULL DEFAULT 0,
        created_at          INTEGER NOT NULL,
        updated_at          INTEGER NOT NULL
    );
    "#
}

/// Migration v5: add reactions, topic_rooms, topic_subscriptions, and topic_posts tables.
pub(crate) fn schema_v5() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS reactions (
        post_hash       TEXT NOT NULL,
        reactor_pubkey  TEXT NOT NULL,
        emoji           TEXT NOT NULL,
        created_at      INTEGER NOT NULL,
        PRIMARY KEY (post_hash, reactor_pubkey)
    );
    CREATE INDEX IF NOT EXISTS idx_reactions_post
        ON reactions(post_hash);

    CREATE TABLE IF NOT EXISTS topic_rooms (
        topic_id    TEXT NOT NULL PRIMARY KEY,
        name        TEXT NOT NULL,
        description TEXT,
        created_by  TEXT NOT NULL,
        created_at  INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS topic_subscriptions (
        topic_id        TEXT NOT NULL,
        user_pubkey     TEXT NOT NULL,
        subscribed_at   INTEGER NOT NULL,
        PRIMARY KEY (topic_id, user_pubkey)
    );
    CREATE INDEX IF NOT EXISTS idx_topic_subs_user
        ON topic_subscriptions(user_pubkey);

    CREATE TABLE IF NOT EXISTS topic_posts (
        topic_id        TEXT NOT NULL,
        content_hash    BLOB NOT NULL,
        created_at      INTEGER NOT NULL,
        PRIMARY KEY (topic_id, content_hash)
    );
    CREATE INDEX IF NOT EXISTS idx_topic_posts_created
        ON topic_posts(topic_id, created_at DESC);
    "#
}

/// Migration v6: add media_attachments and media_chunks tables.
pub(crate) fn schema_v6() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS media_attachments (
        id              TEXT NOT NULL PRIMARY KEY,
        post_hash       BLOB NOT NULL,
        content_hash    TEXT NOT NULL,
        media_type      TEXT NOT NULL,
        mime_type       TEXT NOT NULL,
        file_size       INTEGER NOT NULL,
        width           INTEGER,
        height          INTEGER,
        duration_ms     INTEGER,
        thumbnail_hash  TEXT,
        chunk_count     INTEGER NOT NULL,
        created_at      INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_media_attachments_post
        ON media_attachments(post_hash);

    CREATE TABLE IF NOT EXISTS media_chunks (
        chunk_hash  TEXT NOT NULL PRIMARY KEY,
        media_id    TEXT NOT NULL REFERENCES media_attachments(id) ON DELETE CASCADE,
        chunk_index INTEGER NOT NULL,
        data        BLOB NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_media_chunks_media
        ON media_chunks(media_id, chunk_index);
    "#
}

/// Migration v7: Dead drop mailbox tables for offline message delivery.
pub(crate) fn schema_v7() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS dead_drops (
        message_id  TEXT NOT NULL PRIMARY KEY,
        mailbox_key TEXT NOT NULL,
        sealed_data BLOB NOT NULL,
        deposited_at INTEGER NOT NULL,
        expires_at  INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_dead_drops_mailbox
        ON dead_drops(mailbox_key);
    CREATE INDEX IF NOT EXISTS idx_dead_drops_expires
        ON dead_drops(expires_at);
    "#
}

/// Migration v8: add encrypted message body and epoch-encrypted post body columns.
pub(crate) fn schema_v8() -> &'static str {
    r#"
    ALTER TABLE messages ADD COLUMN encrypted_body BLOB;
    ALTER TABLE messages ADD COLUMN is_encrypted INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE posts ADD COLUMN encrypted_blob BLOB;
    ALTER TABLE posts ADD COLUMN post_epoch_id INTEGER;
    "#
}

/// Migration v9: groups, group members, group bans, group chats, group chat members.
pub(crate) fn schema_v9() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS groups (
        group_id    TEXT NOT NULL PRIMARY KEY,
        name        TEXT NOT NULL UNIQUE,
        handle      TEXT,
        description TEXT,
        avatar_cid  TEXT,
        visibility  TEXT NOT NULL DEFAULT 'public',
        created_by  TEXT NOT NULL,
        created_at  INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_groups_handle
        ON groups(handle) WHERE handle IS NOT NULL;
    CREATE INDEX IF NOT EXISTS idx_groups_visibility
        ON groups(visibility);

    CREATE TABLE IF NOT EXISTS group_members (
        group_id       TEXT NOT NULL,
        member_pubkey  TEXT NOT NULL,
        role           TEXT NOT NULL DEFAULT 'member',
        joined_at      INTEGER NOT NULL,
        invited_by     TEXT,
        PRIMARY KEY (group_id, member_pubkey)
    );
    CREATE INDEX IF NOT EXISTS idx_group_members_pubkey
        ON group_members(member_pubkey);

    CREATE TABLE IF NOT EXISTS group_bans (
        group_id       TEXT NOT NULL,
        banned_pubkey  TEXT NOT NULL,
        banned_by      TEXT NOT NULL,
        reason         TEXT,
        banned_at      INTEGER NOT NULL,
        PRIMARY KEY (group_id, banned_pubkey)
    );

    CREATE TABLE IF NOT EXISTS group_posts (
        group_id       TEXT NOT NULL,
        content_hash   BLOB NOT NULL,
        author_pubkey  TEXT NOT NULL,
        created_at     INTEGER NOT NULL,
        PRIMARY KEY (group_id, content_hash)
    );
    CREATE INDEX IF NOT EXISTS idx_group_posts_created
        ON group_posts(group_id, created_at DESC);

    CREATE TABLE IF NOT EXISTS group_chats (
        chat_id         TEXT NOT NULL PRIMARY KEY,
        name            TEXT,
        is_group_linked INTEGER NOT NULL DEFAULT 0,
        group_id        TEXT,
        created_at      INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_group_chats_group
        ON group_chats(group_id) WHERE group_id IS NOT NULL;

    CREATE TABLE IF NOT EXISTS group_chat_members (
        chat_id        TEXT NOT NULL,
        member_pubkey  TEXT NOT NULL,
        joined_at      INTEGER NOT NULL,
        PRIMARY KEY (chat_id, member_pubkey)
    );
    CREATE INDEX IF NOT EXISTS idx_group_chat_members_pubkey
        ON group_chat_members(member_pubkey);

    CREATE TABLE IF NOT EXISTS group_chat_messages (
        message_id     TEXT NOT NULL PRIMARY KEY,
        chat_id        TEXT NOT NULL,
        sender_pubkey  TEXT NOT NULL,
        body           TEXT,
        created_at     INTEGER NOT NULL,
        expires_at     INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_group_chat_messages_chat
        ON group_chat_messages(chat_id, created_at DESC);
    CREATE INDEX IF NOT EXISTS idx_group_chat_messages_expires
        ON group_chat_messages(expires_at);
    "#
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
