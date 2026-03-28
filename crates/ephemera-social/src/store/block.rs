//! SQLite-backed implementation of [`BlockService`].

use super::*;

#[async_trait::async_trait]
impl BlockService for SqliteSocialServices {
    async fn block(
        &self,
        blocker: &IdentityKey,
        blocked: &IdentityKey,
        reason: Option<&str>,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let blocker_bytes = blocker.as_bytes().to_vec();
        let blocked_bytes = blocked.as_bytes().to_vec();
        let now_secs = Timestamp::now().as_secs() as i64;

        db.conn()
            .execute(
                "INSERT OR REPLACE INTO blocks (blocker_pubkey, blocked_pubkey, created_at, reason)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![blocker_bytes, blocked_bytes, now_secs, reason],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Also remove any existing connection.
        db.conn()
            .execute(
                "DELETE FROM connections WHERE local_pubkey = ?1 AND remote_pubkey = ?2",
                rusqlite::params![blocker_bytes, blocked_bytes],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn unblock(
        &self,
        blocker: &IdentityKey,
        blocked: &IdentityKey,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let blocker_bytes = blocker.as_bytes().to_vec();
        let blocked_bytes = blocked.as_bytes().to_vec();

        db.conn()
            .execute(
                "DELETE FROM blocks WHERE blocker_pubkey = ?1 AND blocked_pubkey = ?2",
                rusqlite::params![blocker_bytes, blocked_bytes],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn mute(
        &self,
        muter: &IdentityKey,
        muted: &IdentityKey,
        expires_at: Option<Timestamp>,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let muter_bytes = muter.as_bytes().to_vec();
        let muted_bytes = muted.as_bytes().to_vec();
        let now_secs = Timestamp::now().as_secs() as i64;
        let expires_secs: Option<i64> = expires_at.map(|t| t.as_secs() as i64);

        db.conn()
            .execute(
                "INSERT OR REPLACE INTO mutes (muter_pubkey, muted_pubkey, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![muter_bytes, muted_bytes, now_secs, expires_secs],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn unmute(&self, muter: &IdentityKey, muted: &IdentityKey) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let muter_bytes = muter.as_bytes().to_vec();
        let muted_bytes = muted.as_bytes().to_vec();

        db.conn()
            .execute(
                "DELETE FROM mutes WHERE muter_pubkey = ?1 AND muted_pubkey = ?2",
                rusqlite::params![muter_bytes, muted_bytes],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn is_blocked(
        &self,
        checker: &IdentityKey,
        target: &IdentityKey,
    ) -> Result<bool, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let checker_bytes = checker.as_bytes().to_vec();
        let target_bytes = target.as_bytes().to_vec();

        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM blocks WHERE blocker_pubkey = ?1 AND blocked_pubkey = ?2",
                rusqlite::params![checker_bytes, target_bytes],
                |row| row.get(0),
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(count > 0)
    }

    async fn is_muted(
        &self,
        checker: &IdentityKey,
        target: &IdentityKey,
    ) -> Result<bool, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let checker_bytes = checker.as_bytes().to_vec();
        let target_bytes = target.as_bytes().to_vec();
        let now_secs = Timestamp::now().as_secs() as i64;

        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM mutes
                 WHERE muter_pubkey = ?1 AND muted_pubkey = ?2
                   AND (expires_at IS NULL OR expires_at > ?3)",
                rusqlite::params![checker_bytes, target_bytes, now_secs],
                |row| row.get(0),
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(count > 0)
    }
}
