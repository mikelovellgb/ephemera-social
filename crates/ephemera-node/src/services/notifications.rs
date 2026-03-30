//! Notification center service.
//!
//! Stores and retrieves user notifications (connection requests received,
//! connections accepted, new messages, mentions). Notifications are persisted
//! in SQLite so they survive restarts.

use ephemera_store::MetadataDb;
use serde_json::Value;
use std::sync::Mutex;

/// Notification service backed by the SQLite `notifications` table.
pub struct NotificationService;

impl NotificationService {
    /// Insert a new notification.
    ///
    /// `notification_type` is one of: `connection_request`, `connection_accepted`,
    /// `new_message`, `mention`.
    pub fn insert(
        metadata_db: &Mutex<MetadataDb>,
        notification_type: &str,
        from_pubkey: Option<&[u8]>,
        display_name: Option<&str>,
        preview: Option<&str>,
        post_hash: Option<&[u8]>,
    ) -> Result<String, String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs() as i64;

        // Generate a unique notification ID from type + timestamp + random.
        let id_input = format!(
            "{}:{}:{}",
            notification_type,
            now,
            rand::random::<u64>(),
        );
        let id_hash = blake3::hash(id_input.as_bytes());
        let notification_id = hex::encode(&id_hash.as_bytes()[..16]);

        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO notifications
                 (notification_id, notification_type, from_pubkey, display_name, preview, post_hash, is_read, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
                rusqlite::params![
                    notification_id,
                    notification_type,
                    from_pubkey,
                    display_name,
                    preview,
                    post_hash,
                    now,
                ],
            )
            .map_err(|e| format!("insert notification: {e}"))?;

        Ok(notification_id)
    }

    /// List unread notifications (most recent first).
    pub fn list_unread(
        metadata_db: &Mutex<MetadataDb>,
        limit: u32,
    ) -> Result<Value, String> {
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT notification_id, notification_type, from_pubkey, display_name, preview, post_hash, is_read, created_at
                 FROM notifications
                 WHERE is_read = 0
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| format!("prepare: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params![limit], |row| {
                let id: String = row.get(0)?;
                let ntype: String = row.get(1)?;
                let from_pubkey: Option<Vec<u8>> = row.get(2)?;
                let display_name: Option<String> = row.get(3)?;
                let preview: Option<String> = row.get(4)?;
                let post_hash: Option<Vec<u8>> = row.get(5)?;
                let is_read: i64 = row.get(6)?;
                let created_at: i64 = row.get(7)?;

                Ok(serde_json::json!({
                    "notification_id": id,
                    "type": ntype,
                    "from_pubkey": from_pubkey.map(|b| hex::encode(b)),
                    "display_name": display_name,
                    "preview": preview,
                    "post_hash": post_hash.map(|b| hex::encode(b)),
                    "is_read": is_read != 0,
                    "created_at": created_at,
                }))
            })
            .map_err(|e| format!("query: {e}"))?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|e| format!("row: {e}"))?);
        }

        Ok(serde_json::json!({ "notifications": items }))
    }

    /// Count unread notifications.
    pub fn count_unread(metadata_db: &Mutex<MetadataDb>) -> Result<Value, String> {
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM notifications WHERE is_read = 0",
                [],
                |row| row.get(0),
            )
            .map_err(|e| format!("count: {e}"))?;

        Ok(serde_json::json!({ "unread_count": count }))
    }

    /// Mark a single notification as read.
    pub fn mark_read(
        metadata_db: &Mutex<MetadataDb>,
        notification_id: &str,
    ) -> Result<Value, String> {
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let changed = db
            .conn()
            .execute(
                "UPDATE notifications SET is_read = 1 WHERE notification_id = ?1",
                rusqlite::params![notification_id],
            )
            .map_err(|e| format!("mark_read: {e}"))?;

        Ok(serde_json::json!({ "marked": changed > 0 }))
    }

    /// Mark all notifications as read.
    pub fn mark_all_read(metadata_db: &Mutex<MetadataDb>) -> Result<Value, String> {
        let db = metadata_db.lock().map_err(|e| format!("lock: {e}"))?;
        let changed = db
            .conn()
            .execute("UPDATE notifications SET is_read = 1 WHERE is_read = 0", [])
            .map_err(|e| format!("mark_all_read: {e}"))?;

        Ok(serde_json::json!({ "marked": changed }))
    }
}
