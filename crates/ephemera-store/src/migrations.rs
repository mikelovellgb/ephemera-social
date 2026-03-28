//! Schema migration system for the SQLite metadata database.
//!
//! Tracks applied schema versions in the `schema_version` table and
//! applies pending migrations sequentially on startup. Each migration
//! is an idempotent SQL script.

use crate::metadata::{self, MetadataDb};
use crate::StoreError;

/// A single migration step.
struct Migration {
    version: i64,
    description: &'static str,
    up: &'static str,
}

/// All known migrations, in order.
fn all_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "Initial schema",
            up: metadata::schema_v1(),
        },
        Migration {
            version: 2,
            description: "Add message and initiator columns to connections",
            up: metadata::schema_v2(),
        },
        Migration {
            version: 3,
            description: "Add message_requests table for stranger DM handling",
            up: metadata::schema_v3(),
        },
        Migration {
            version: 4,
            description: "Add ratchet_sessions table for Double Ratchet state",
            up: metadata::schema_v4(),
        },
        Migration {
            version: 5,
            description: "Add reactions, topic_rooms, topic_subscriptions, topic_posts",
            up: metadata::schema_v5(),
        },
        Migration {
            version: 6,
            description: "Add media_attachments and media_chunks tables",
            up: metadata::schema_v6(),
        },
        Migration {
            version: 7,
            description: "Add dead_drops table for offline message delivery",
            up: metadata::schema_v7(),
        },
        Migration {
            version: 8,
            description: "Add encrypted message body and epoch-encrypted post columns",
            up: metadata::schema_v8(),
        },
        Migration {
            version: 9,
            description: "Add groups, group_members, group_bans, group_posts, group_chats, group_chat_members, group_chat_messages",
            up: metadata::schema_v9(),
        },
    ]
}

/// Run all pending migrations against `db`.
///
/// Creates the `schema_version` table if it does not exist, then applies
/// every migration whose version is greater than the current maximum.
pub(crate) fn run_migrations(db: &mut MetadataDb) -> Result<(), StoreError> {
    // Ensure the schema_version table exists (bootstrap case).
    db.conn().execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version     INTEGER NOT NULL,
            applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            description TEXT
        );",
    )?;

    let current_version = current_version(db)?;

    for migration in all_migrations() {
        if migration.version <= current_version {
            continue;
        }
        tracing::info!(
            version = migration.version,
            desc = migration.description,
            "applying migration"
        );
        let tx = db.conn_mut().transaction()?;
        tx.execute_batch(migration.up)?;
        tx.execute(
            "INSERT INTO schema_version (version, description) VALUES (?1, ?2)",
            rusqlite::params![migration.version, migration.description],
        )?;
        tx.commit()?;
    }

    Ok(())
}

/// Return the highest applied schema version, or 0 if none.
fn current_version(db: &MetadataDb) -> Result<i64, StoreError> {
    let version: i64 = db.conn().query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

/// Roll back to a target version by applying down migrations.
///
/// Currently a placeholder -- down migrations will be added when the
/// schema evolves beyond version 1.
#[allow(dead_code)]
pub(crate) fn rollback_to(_db: &mut MetadataDb, _target_version: i64) -> Result<(), StoreError> {
    // No down migrations yet.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let mut db = MetadataDb::open_in_memory().unwrap();
        // Migrations already ran in open_in_memory. Running again should be a no-op.
        run_migrations(&mut db).unwrap();
        let v = current_version(&db).unwrap();
        assert_eq!(v, 9);
    }

    #[test]
    fn current_version_after_bootstrap() {
        let db = MetadataDb::open_in_memory().unwrap();
        let v = current_version(&db).unwrap();
        assert_eq!(v, 9);
    }
}
