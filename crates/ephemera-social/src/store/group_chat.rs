//! SQLite-backed group chat storage.
//!
//! Implements persistence for both group-linked chats and standalone
//! private group chats.

use super::*;
use crate::group_chat::{
    generate_chat_id, GroupChat, GroupChatMessage, GroupChatSummary, MAX_PRIVATE_CHAT_MEMBERS,
};

impl SqliteSocialServices {
    // ── Group chat CRUD ───────────────────────────────────────────

    /// Create a group-linked chat for an existing group.
    pub fn create_group_linked_chat(
        &self,
        group_id: &str,
        created_at: i64,
    ) -> Result<GroupChat, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Check that the group exists.
        let group_name: String = db
            .conn()
            .query_row(
                "SELECT name FROM groups WHERE group_id = ?1",
                rusqlite::params![group_id],
                |row| row.get(0),
            )
            .map_err(|_| SocialError::NotFound(format!("group {group_id} not found")))?;

        // Check if a linked chat already exists.
        let existing: Option<String> = db
            .conn()
            .query_row(
                "SELECT chat_id FROM group_chats WHERE group_id = ?1 AND is_group_linked = 1",
                rusqlite::params![group_id],
                |row| row.get(0),
            )
            .ok();

        if let Some(chat_id) = existing {
            return Ok(GroupChat {
                chat_id,
                name: Some(group_name),
                is_group_linked: true,
                group_id: Some(group_id.to_string()),
                created_at: Timestamp::from_secs(created_at as u64),
            });
        }

        let chat_id = generate_chat_id(group_id, created_at as u64);

        db.conn()
            .execute(
                "INSERT INTO group_chats (chat_id, name, is_group_linked, group_id, created_at)
                 VALUES (?1, ?2, 1, ?3, ?4)",
                rusqlite::params![chat_id, group_name, group_id, created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(GroupChat {
            chat_id,
            name: Some(group_name),
            is_group_linked: true,
            group_id: Some(group_id.to_string()),
            created_at: Timestamp::from_secs(created_at as u64),
        })
    }

    /// Create a private group chat with initial members.
    pub fn create_private_group_chat(
        &self,
        name: Option<&str>,
        creator_pubkey: &str,
        member_pubkeys: &[&str],
        created_at: i64,
    ) -> Result<GroupChat, SocialError> {
        let total_members = member_pubkeys.len() + 1; // +1 for creator
        if total_members > MAX_PRIVATE_CHAT_MEMBERS {
            return Err(SocialError::Validation(format!(
                "group chat cannot have more than {MAX_PRIVATE_CHAT_MEMBERS} members"
            )));
        }

        let chat_id = generate_chat_id(creator_pubkey, created_at as u64);

        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "INSERT INTO group_chats (chat_id, name, is_group_linked, group_id, created_at)
                 VALUES (?1, ?2, 0, NULL, ?3)",
                rusqlite::params![chat_id, name, created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Add creator.
        db.conn()
            .execute(
                "INSERT INTO group_chat_members (chat_id, member_pubkey, joined_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![chat_id, creator_pubkey, created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Add other members.
        for pk in member_pubkeys {
            if *pk != creator_pubkey {
                db.conn()
                    .execute(
                        "INSERT OR IGNORE INTO group_chat_members (chat_id, member_pubkey, joined_at)
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![chat_id, pk, created_at],
                    )
                    .map_err(|e| SocialError::Storage(e.to_string()))?;
            }
        }

        Ok(GroupChat {
            chat_id,
            name: name.map(String::from),
            is_group_linked: false,
            group_id: None,
            created_at: Timestamp::from_secs(created_at as u64),
        })
    }

    /// Add a member to a private group chat.
    ///
    /// The added user must not have blocked the adder. The `adder_pubkey`
    /// is the hex-encoded pubkey of the user performing the add.
    pub fn add_chat_member(
        &self,
        chat_id: &str,
        member_pubkey: &str,
        adder_pubkey: Option<&str>,
        joined_at: i64,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Check that the adder is a current member (if provided).
        if let Some(adder) = adder_pubkey {
            let adder_is_member: bool = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM group_chat_members WHERE chat_id = ?1 AND member_pubkey = ?2",
                    rusqlite::params![chat_id, adder],
                    |row| row.get::<_, i64>(0).map(|c| c > 0),
                )
                .unwrap_or(false);

            if !adder_is_member {
                return Err(SocialError::PermissionDenied(
                    "only current members can add people to the chat".into(),
                ));
            }

            // Check if the target user has blocked the adder.
            if let (Ok(target_bytes), Ok(adder_bytes)) =
                (hex::decode(member_pubkey), hex::decode(adder))
            {
                let is_blocked: bool = db
                    .conn()
                    .query_row(
                        "SELECT COUNT(*) FROM blocks WHERE blocker_pubkey = ?1 AND blocked_pubkey = ?2",
                        rusqlite::params![target_bytes, adder_bytes],
                        |row| row.get::<_, i64>(0).map(|c| c > 0),
                    )
                    .unwrap_or(false);

                if is_blocked {
                    return Err(SocialError::PermissionDenied(
                        "cannot add this user (they have blocked you)".into(),
                    ));
                }
            }
        }

        // Check current member count.
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM group_chat_members WHERE chat_id = ?1",
                rusqlite::params![chat_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if count as usize >= MAX_PRIVATE_CHAT_MEMBERS {
            return Err(SocialError::Validation(format!(
                "group chat is full ({MAX_PRIVATE_CHAT_MEMBERS} members max)"
            )));
        }

        db.conn()
            .execute(
                "INSERT OR IGNORE INTO group_chat_members (chat_id, member_pubkey, joined_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![chat_id, member_pubkey, joined_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Remove self from a group chat.
    pub fn leave_chat(
        &self,
        chat_id: &str,
        member_pubkey: &str,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "DELETE FROM group_chat_members WHERE chat_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![chat_id, member_pubkey],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    // ── Chat messages ─────────────────────────────────────────────

    /// Send a message to a group chat.
    ///
    /// The sender must be a current member of the chat.
    pub fn send_group_chat_message(
        &self,
        message_id: &str,
        chat_id: &str,
        sender_pubkey: &str,
        body: Option<&str>,
        created_at: i64,
        expires_at: i64,
    ) -> Result<GroupChatMessage, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Verify sender is a member of this chat.
        let is_member: bool = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM group_chat_members WHERE chat_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![chat_id, sender_pubkey],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if !is_member {
            return Err(SocialError::PermissionDenied(
                "not a member of this chat".into(),
            ));
        }

        db.conn()
            .execute(
                "INSERT INTO group_chat_messages (message_id, chat_id, sender_pubkey, body, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![message_id, chat_id, sender_pubkey, body, created_at, expires_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(GroupChatMessage {
            message_id: message_id.to_string(),
            chat_id: chat_id.to_string(),
            sender_pubkey: sender_pubkey.to_string(),
            body: body.map(String::from),
            created_at: Timestamp::from_secs(created_at as u64),
            expires_at: Timestamp::from_secs(expires_at as u64),
        })
    }

    /// Get messages in a group chat, paginated.
    ///
    /// The reader must be a current member of the chat. Pass `None` for
    /// `reader_pubkey` only in internal/admin contexts.
    pub fn get_chat_messages(
        &self,
        chat_id: &str,
        limit: u32,
        before: Option<i64>,
        reader_pubkey: Option<&str>,
    ) -> Result<Vec<GroupChatMessage>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // If a reader pubkey is provided, verify membership.
        if let Some(reader) = reader_pubkey {
            let is_member: bool = db
                .conn()
                .query_row(
                    "SELECT COUNT(*) FROM group_chat_members WHERE chat_id = ?1 AND member_pubkey = ?2",
                    rusqlite::params![chat_id, reader],
                    |row| row.get::<_, i64>(0).map(|c| c > 0),
                )
                .unwrap_or(false);

            if !is_member {
                return Err(SocialError::PermissionDenied(
                    "not a member of this chat".into(),
                ));
            }
        }

        let fetch_limit = limit.min(200) as i64;

        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match before {
            Some(ts) => (
                "SELECT message_id, chat_id, sender_pubkey, body, created_at, expires_at
                 FROM group_chat_messages
                 WHERE chat_id = ?1 AND created_at < ?2
                 ORDER BY created_at DESC LIMIT ?3",
                vec![
                    Box::new(chat_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(ts),
                    Box::new(fetch_limit),
                ],
            ),
            None => (
                "SELECT message_id, chat_id, sender_pubkey, body, created_at, expires_at
                 FROM group_chat_messages
                 WHERE chat_id = ?1
                 ORDER BY created_at DESC LIMIT ?2",
                vec![
                    Box::new(chat_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(fetch_limit),
                ],
            ),
        };

        let mut stmt = db
            .conn()
            .prepare(sql)
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| &**p).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(GroupChatMessage {
                    message_id: row.get(0)?,
                    chat_id: row.get(1)?,
                    sender_pubkey: row.get(2)?,
                    body: row.get(3)?,
                    created_at: Timestamp::from_secs(row.get::<_, i64>(4)? as u64),
                    expires_at: Timestamp::from_secs(row.get::<_, i64>(5)? as u64),
                })
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row.map_err(|e| SocialError::Storage(e.to_string()))?);
        }
        Ok(messages)
    }

    // ── Chat queries ──────────────────────────────────────────────

    /// List all group chats the user is a member of.
    pub fn list_my_group_chats(
        &self,
        member_pubkey: &str,
    ) -> Result<Vec<GroupChatSummary>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut stmt = db
            .conn()
            .prepare(
                "SELECT gc.chat_id, gc.name, gc.is_group_linked, gc.group_id, gc.created_at,
                        (SELECT COUNT(*) FROM group_chat_members WHERE chat_id = gc.chat_id) AS member_count,
                        (SELECT body FROM group_chat_messages WHERE chat_id = gc.chat_id ORDER BY created_at DESC LIMIT 1),
                        (SELECT created_at FROM group_chat_messages WHERE chat_id = gc.chat_id ORDER BY created_at DESC LIMIT 1)
                 FROM group_chats gc
                 JOIN group_chat_members gcm ON gc.chat_id = gcm.chat_id
                 WHERE gcm.member_pubkey = ?1
                 ORDER BY COALESCE(
                     (SELECT created_at FROM group_chat_messages WHERE chat_id = gc.chat_id ORDER BY created_at DESC LIMIT 1),
                     gc.created_at
                 ) DESC",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![member_pubkey], |row| {
                let last_msg_at: Option<i64> = row.get(7)?;
                Ok(GroupChatSummary {
                    chat: GroupChat {
                        chat_id: row.get(0)?,
                        name: row.get(1)?,
                        is_group_linked: row.get::<_, i64>(2)? != 0,
                        group_id: row.get(3)?,
                        created_at: Timestamp::from_secs(row.get::<_, i64>(4)? as u64),
                    },
                    member_count: row.get::<_, i64>(5)? as u32,
                    last_message: row.get(6)?,
                    last_message_at: last_msg_at.map(|ts| Timestamp::from_secs(ts as u64)),
                })
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut chats = Vec::new();
        for row in rows {
            chats.push(row.map_err(|e| SocialError::Storage(e.to_string()))?);
        }
        Ok(chats)
    }
}
