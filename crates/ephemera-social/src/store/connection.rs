//! SQLite-backed implementation of [`ConnectionService`].

use super::*;

#[async_trait::async_trait]
impl ConnectionService for SqliteSocialServices {
    async fn request(
        &self,
        from: &IdentityKey,
        to: &IdentityKey,
        message: Option<&str>,
    ) -> Result<Connection, ConnectionError> {
        if let Some(msg) = message {
            if msg.len() > MAX_CONNECTION_MESSAGE_LEN {
                return Err(ConnectionError::MessageTooLong { len: msg.len() });
            }
        }

        let db = self
            .db
            .lock()
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;
        let from_bytes = from.as_bytes().to_vec();
        let to_bytes = to.as_bytes().to_vec();
        let now = Timestamp::now();
        let now_secs = now.as_secs() as i64;

        // Check for existing connection.
        let existing: Option<String> = db
            .conn()
            .query_row(
                "SELECT status FROM connections WHERE local_pubkey = ?1 AND remote_pubkey = ?2",
                rusqlite::params![from_bytes, to_bytes],
                |row| row.get(0),
            )
            .ok();

        if let Some(status_str) = existing {
            let status = Self::str_to_status(&status_str);
            return Err(ConnectionError::AlreadyExists { status });
        }

        // Insert the outgoing request.
        db.conn()
            .execute(
                "INSERT INTO connections (local_pubkey, remote_pubkey, status, created_at, updated_at, message, initiator_pubkey)
                 VALUES (?1, ?2, 'pending_outgoing', ?3, ?3, ?4, ?1)",
                rusqlite::params![from_bytes, to_bytes, now_secs, message],
            )
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;

        Ok(Connection {
            initiator: *from,
            responder: *to,
            status: ConnectionStatus::PendingOutgoing,
            created_at: now,
            updated_at: now,
            message: message.map(|s| s.to_string()),
        })
    }

    async fn accept(
        &self,
        local: &IdentityKey,
        remote: &IdentityKey,
    ) -> Result<Connection, ConnectionError> {
        let db = self
            .db
            .lock()
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;
        let local_bytes = local.as_bytes().to_vec();
        let remote_bytes = remote.as_bytes().to_vec();
        let now = Timestamp::now();
        let now_secs = now.as_secs() as i64;

        // Update local side to connected.
        db.conn()
            .execute(
                "UPDATE connections SET status = 'connected', updated_at = ?3
                 WHERE local_pubkey = ?1 AND remote_pubkey = ?2",
                rusqlite::params![local_bytes, remote_bytes, now_secs],
            )
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;

        // Also update the other direction to connected.
        db.conn()
            .execute(
                "UPDATE connections SET status = 'connected', updated_at = ?3
                 WHERE local_pubkey = ?1 AND remote_pubkey = ?2",
                rusqlite::params![remote_bytes, local_bytes, now_secs],
            )
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;

        Ok(Connection {
            initiator: *remote,
            responder: *local,
            status: ConnectionStatus::Active,
            created_at: now,
            updated_at: now,
            message: None,
        })
    }

    async fn reject(
        &self,
        local: &IdentityKey,
        remote: &IdentityKey,
    ) -> Result<(), ConnectionError> {
        let db = self
            .db
            .lock()
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;
        let local_bytes = local.as_bytes().to_vec();
        let remote_bytes = remote.as_bytes().to_vec();

        // Delete the incoming request on our side.
        db.conn()
            .execute(
                "DELETE FROM connections WHERE local_pubkey = ?1 AND remote_pubkey = ?2",
                rusqlite::params![local_bytes, remote_bytes],
            )
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn remove(
        &self,
        local: &IdentityKey,
        remote: &IdentityKey,
    ) -> Result<(), ConnectionError> {
        let db = self
            .db
            .lock()
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;
        let local_bytes = local.as_bytes().to_vec();
        let remote_bytes = remote.as_bytes().to_vec();

        // Delete both directions.
        db.conn()
            .execute(
                "DELETE FROM connections WHERE local_pubkey = ?1 AND remote_pubkey = ?2",
                rusqlite::params![local_bytes, remote_bytes],
            )
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "DELETE FROM connections WHERE local_pubkey = ?1 AND remote_pubkey = ?2",
                rusqlite::params![remote_bytes, local_bytes],
            )
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;

        Ok(())
    }

    async fn list(
        &self,
        identity: &IdentityKey,
        status_filter: Option<ConnectionStatus>,
    ) -> Result<Vec<Connection>, ConnectionError> {
        let db = self
            .db
            .lock()
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;
        let local_bytes = identity.as_bytes().to_vec();

        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match status_filter {
            Some(status) => {
                let status_str = Self::status_to_str(status);
                (
                    "SELECT local_pubkey, remote_pubkey, status, created_at, updated_at, message, initiator_pubkey
                     FROM connections WHERE local_pubkey = ?1 AND status = ?2
                     ORDER BY updated_at DESC",
                    vec![
                        Box::new(local_bytes.clone()) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(status_str.to_string()),
                    ],
                )
            }
            None => (
                "SELECT local_pubkey, remote_pubkey, status, created_at, updated_at, message, initiator_pubkey
                 FROM connections WHERE local_pubkey = ?1
                 ORDER BY updated_at DESC",
                vec![Box::new(local_bytes.clone()) as Box<dyn rusqlite::types::ToSql>],
            ),
        };

        let mut stmt = db
            .conn()
            .prepare(sql)
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                let local_pk: Vec<u8> = row.get(0)?;
                let remote_pk: Vec<u8> = row.get(1)?;
                let status_str: String = row.get(2)?;
                let created_at: i64 = row.get(3)?;
                let updated_at: i64 = row.get(4)?;
                let message: Option<String> = row.get(5)?;
                let initiator_pk: Option<Vec<u8>> = row.get(6)?;
                Ok((
                    local_pk,
                    remote_pk,
                    status_str,
                    created_at,
                    updated_at,
                    message,
                    initiator_pk,
                ))
            })
            .map_err(|e| ConnectionError::Storage(e.to_string()))?;

        let mut connections = Vec::new();
        for row in rows {
            let (local_pk, remote_pk, status_str, created_at, updated_at, message, initiator_pk) =
                row.map_err(|e| ConnectionError::Storage(e.to_string()))?;
            let status = Self::str_to_status(&status_str);
            let local_key = bytes_to_identity_key(&local_pk);
            let remote_key = bytes_to_identity_key(&remote_pk);
            let initiator = initiator_pk
                .as_deref()
                .map(bytes_to_identity_key)
                .unwrap_or(local_key);
            let responder = if initiator == local_key {
                remote_key
            } else {
                local_key
            };
            connections.push(Connection {
                initiator,
                responder,
                status,
                created_at: Timestamp::from_secs(created_at as u64),
                updated_at: Timestamp::from_secs(updated_at as u64),
                message,
            });
        }

        Ok(connections)
    }
}

/// Process an incoming connection request on the receiver's side.
///
/// Returns `Some(Connection)` if the request was stored, or `None` if the
/// sender is blocked (silently discarded).
pub fn receive_connection_request(
    db: &MetadataDb,
    receiver: &IdentityKey,
    sender: &IdentityKey,
    message: Option<&str>,
) -> Result<Option<Connection>, ConnectionError> {
    let receiver_bytes = receiver.as_bytes().to_vec();
    let sender_bytes = sender.as_bytes().to_vec();
    let now = Timestamp::now();
    let now_secs = now.as_secs() as i64;

    // Check if the sender is blocked by the receiver.
    let is_blocked: bool = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM blocks WHERE blocker_pubkey = ?1 AND blocked_pubkey = ?2",
            rusqlite::params![receiver_bytes, sender_bytes],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .unwrap_or(false);

    if is_blocked {
        return Ok(None);
    }

    // Insert the incoming request on the receiver's side.
    db.conn()
        .execute(
            "INSERT OR REPLACE INTO connections (local_pubkey, remote_pubkey, status, created_at, updated_at, message, initiator_pubkey)
             VALUES (?1, ?2, 'pending_incoming', ?3, ?3, ?4, ?2)",
            rusqlite::params![receiver_bytes, sender_bytes, now_secs, message],
        )
        .map_err(|e| ConnectionError::Storage(e.to_string()))?;

    Ok(Some(Connection {
        initiator: *sender,
        responder: *receiver,
        status: ConnectionStatus::PendingIncoming,
        created_at: now,
        updated_at: now,
        message: message.map(|s| s.to_string()),
    }))
}
