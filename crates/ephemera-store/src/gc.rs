//! Garbage collection for expired content.
//!
//! Runs a four-phase sweep: expire content, purge tombstones, destroy
//! old epoch keys, and clean up orphaned blobs/partitions.

use crate::content_store::ContentStore;
use crate::metadata::MetadataDb;
use crate::StoreError;
use ephemera_events::{Event, EventBus};
use ephemera_types::ContentId;
use std::time::Duration;

/// A clock function that returns the current time as Unix seconds.
///
/// The default is the system wall clock. Inject a custom function for
/// deterministic testing without waiting for real time to pass.
pub type ClockFn = fn() -> i64;

/// Return the current wall-clock time as Unix seconds (default clock).
fn wall_clock_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs() as i64
}

/// Configuration for the garbage collector.
pub struct GcConfig {
    /// How often the GC sweep runs.
    pub interval: Duration,
    /// Maximum number of items to process per sweep (avoid long pauses).
    pub batch_size: u32,
    /// Time source used to determine "now". Defaults to wall clock.
    /// Replace with a fake clock for deterministic testing.
    pub clock: ClockFn,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(60),
            batch_size: 1000,
            clock: wall_clock_now,
        }
    }
}

/// Report returned after a GC sweep.
#[derive(Debug, Default)]
pub struct GcReport {
    pub posts_deleted: u64,
    pub messages_deleted: u64,
    pub tombstones_purged: u64,
    pub epoch_keys_destroyed: u64,
    pub orphaned_blobs_deleted: u64,
    pub day_partitions_removed: u64,
    pub bytes_freed: u64,
}

impl GcReport {
    /// Total number of items cleaned across all phases.
    #[must_use]
    pub fn total_items(&self) -> u64 {
        self.posts_deleted
            + self.messages_deleted
            + self.tombstones_purged
            + self.epoch_keys_destroyed
            + self.orphaned_blobs_deleted
            + self.day_partitions_removed
    }
}

/// Background garbage collector that removes expired content.
///
/// Call [`sweep`](Self::sweep) periodically or wrap in a tokio task.
pub struct GarbageCollector {
    config: GcConfig,
}

impl GarbageCollector {
    #[must_use]
    pub fn new(config: GcConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(GcConfig::default())
    }

    /// The configured sweep interval.
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.config.interval
    }

    /// Run one GC sweep: all four phases.
    pub fn sweep(&self, db: &MetadataDb, content: &ContentStore) -> Result<GcReport, StoreError> {
        self.sweep_with_events(db, content, None)
    }

    /// Run one GC sweep with optional event emission.
    pub fn sweep_with_events(
        &self,
        db: &MetadataDb,
        content: &ContentStore,
        event_bus: Option<&EventBus>,
    ) -> Result<GcReport, StoreError> {
        let mut report = GcReport::default();
        let now_secs = (self.config.clock)();

        self.phase1_expire_content(db, content, now_secs, event_bus, &mut report)?;
        self.phase2_purge_tombstones(db, now_secs, &mut report)?;
        self.phase3_destroy_epoch_keys(db, now_secs, &mut report)?;
        self.phase4_sweep_orphaned_content(db, content, &mut report)?;

        tracing::info!(
            posts_deleted = report.posts_deleted,
            messages_deleted = report.messages_deleted,
            tombstones_purged = report.tombstones_purged,
            epoch_keys_destroyed = report.epoch_keys_destroyed,
            orphaned_blobs = report.orphaned_blobs_deleted,
            partitions_removed = report.day_partitions_removed,
            "GC sweep completed"
        );

        // Emit completion event.
        if let Some(bus) = event_bus {
            bus.emit(Event::GarbageCollectionCompleted {
                items_removed: report.total_items(),
                bytes_freed: report.bytes_freed,
            });
        }

        Ok(report)
    }

    /// Phase 1: Find expired, non-tombstoned posts. Delete their content
    /// blobs and mark metadata as tombstone.
    fn phase1_expire_content(
        &self,
        db: &MetadataDb,
        content: &ContentStore,
        now_secs: i64,
        event_bus: Option<&EventBus>,
        report: &mut GcReport,
    ) -> Result<(), StoreError> {
        // 1a. Find expired posts with their blob hashes and content hashes.
        let mut stmt = db.conn().prepare(
            "SELECT content_hash, blob_hash FROM posts
             WHERE expires_at < ?1 AND is_tombstone = 0
             LIMIT ?2",
        )?;
        let expired_entries: Vec<(Vec<u8>, Option<String>)> = stmt
            .query_map(rusqlite::params![now_secs, self.config.batch_size], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Delete content blobs for expired posts and emit events.
        for (hash_bytes, blob_hash) in &expired_entries {
            let hex_key = match blob_hash {
                Some(bh) => bh.clone(),
                None => hex::encode(hash_bytes),
            };
            if content.delete(&hex_key)? {
                report.posts_deleted += 1;
            }

            // Emit PostExpired event.
            if let Some(bus) = event_bus {
                if hash_bytes.len() == 33 {
                    if let Some(content_id) = ContentId::from_wire_bytes(hash_bytes) {
                        bus.emit(Event::PostExpired { content_id });
                    }
                } else if hash_bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(hash_bytes);
                    bus.emit(Event::PostExpired {
                        content_id: ContentId::from_hash(arr),
                    });
                }
            }
        }

        // 1a-ii. Delete media chunks and attachments for expired posts (M-03).
        for (hash_bytes, _) in &expired_entries {
            // Delete media chunks for all attachments of this post.
            let _ = db.conn().execute(
                "DELETE FROM media_chunks WHERE media_id IN (
                     SELECT id FROM media_attachments WHERE post_hash = ?1
                 )",
                rusqlite::params![hash_bytes],
            );
            // Delete the thumbnail blobs from content store.
            if let Ok(mut thumb_stmt) = db.conn().prepare(
                "SELECT thumbnail_hash FROM media_attachments
                 WHERE post_hash = ?1 AND thumbnail_hash IS NOT NULL",
            ) {
                if let Ok(rows) = thumb_stmt.query_map(rusqlite::params![hash_bytes], |row| row.get::<_, String>(0)) {
                    for th in rows.flatten() {
                        let _ = content.delete(&th);
                    }
                }
            }
            // Delete media attachment metadata.
            let _ = db.conn().execute(
                "DELETE FROM media_attachments WHERE post_hash = ?1",
                rusqlite::params![hash_bytes],
            );
        }

        // 1b. Mark ONLY the batch-selected expired posts as tombstones
        // (not all expired posts -- that would cause a mismatch with blob deletion).
        if !expired_entries.is_empty() {
            let placeholders: Vec<String> = (1..=expired_entries.len())
                .map(|i| format!("?{}", i + 1))
                .collect();
            let in_clause = placeholders.join(", ");
            let sql = format!(
                "UPDATE posts SET is_tombstone = 1, tombstone_at = ?1
                 WHERE content_hash IN ({in_clause})"
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            params.push(Box::new(now_secs));
            for (hash_bytes, _) in &expired_entries {
                params.push(Box::new(hash_bytes.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            db.conn().execute(&sql, param_refs.as_slice())?;
        }

        // 1c. Delete expired messages.
        let msg_deleted = db.conn().execute(
            "DELETE FROM messages WHERE expires_at < ?1",
            rusqlite::params![now_secs],
        )?;
        report.messages_deleted = msg_deleted as u64;

        Ok(())
    }

    /// Phase 2: Purge old tombstones (retained for 3x their original TTL).
    fn phase2_purge_tombstones(
        &self,
        db: &MetadataDb,
        now_secs: i64,
        report: &mut GcReport,
    ) -> Result<(), StoreError> {
        let purged = db.conn().execute(
            "DELETE FROM posts
             WHERE is_tombstone = 1
               AND tombstone_at IS NOT NULL
               AND tombstone_at + (ttl_seconds * 3) < ?1",
            rusqlite::params![now_secs],
        )?;
        report.tombstones_purged = purged as u64;
        Ok(())
    }

    /// Phase 3: Find epoch keys older than 30 days and mark them as destroyed
    /// in the database.
    fn phase3_destroy_epoch_keys(
        &self,
        db: &MetadataDb,
        now_secs: i64,
        report: &mut GcReport,
    ) -> Result<(), StoreError> {
        let destroyed = db.conn().execute(
            "UPDATE epoch_keys SET is_deleted = 1, deleted_at = ?1
             WHERE is_deleted = 0 AND expires_at < ?1",
            rusqlite::params![now_secs],
        )?;
        report.epoch_keys_destroyed = destroyed as u64;
        Ok(())
    }

    /// Phase 4: Find any remaining content blobs from destroyed epochs
    /// and delete them. Also clean up empty day partitions.
    fn phase4_sweep_orphaned_content(
        &self,
        db: &MetadataDb,
        content: &ContentStore,
        report: &mut GcReport,
    ) -> Result<(), StoreError> {
        // 4a. Find posts belonging to destroyed epochs and delete their blobs.
        let mut stmt = db.conn().prepare(
            "SELECT p.content_hash, p.blob_hash FROM posts p
             INNER JOIN epoch_keys e ON p.epoch_number = e.epoch_number
             WHERE e.is_deleted = 1
             LIMIT ?1",
        )?;
        let orphaned: Vec<(Vec<u8>, Option<String>)> = stmt
            .query_map(rusqlite::params![self.config.batch_size], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        for (hash_bytes, blob_hash) in &orphaned {
            let hex_key = match blob_hash {
                Some(bh) => bh.clone(),
                None => hex::encode(hash_bytes),
            };
            if content.delete(&hex_key)? {
                report.orphaned_blobs_deleted += 1;
            }
        }

        // Delete the metadata rows for orphaned posts.
        db.conn().execute(
            "DELETE FROM posts WHERE epoch_number IN (
                SELECT epoch_number FROM epoch_keys WHERE is_deleted = 1
             )",
            [],
        )?;

        // 4b. Clean up empty day partition directories.
        if let Ok(partitions) = content.list_day_partitions() {
            for date in &partitions {
                if content.is_day_partition_empty(date)? && content.delete_day_partition(date)? {
                    report.day_partitions_removed += 1;
                }
            }
        }

        Ok(())
    }
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
#[path = "gc_tests.rs"]
mod tests;
