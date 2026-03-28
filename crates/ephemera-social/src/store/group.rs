//! SQLite-backed group storage.
//!
//! Implements group CRUD, membership, role management, bans, and
//! group feed queries.

use super::*;
use crate::feed::FeedCursor;
use crate::group::{
    group_id_from_name, validate_group_description, validate_group_name, Group, GroupInfo,
    GroupMember, GroupRole, GroupVisibility,
};
use crate::handle_validation::validate_handle_format;

impl SqliteSocialServices {
    // ── Group CRUD ─────────────────────────────────────────────────

    /// Create a new group. The creator becomes the owner.
    pub fn create_group(
        &self,
        name: &str,
        description: Option<&str>,
        visibility: GroupVisibility,
        creator_pubkey: &str,
        created_at: i64,
    ) -> Result<Group, SocialError> {
        validate_group_name(name)?;
        if let Some(desc) = description {
            validate_group_description(desc)?;
        }

        let group_id = group_id_from_name(name);

        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let inserted = db
            .conn()
            .execute(
                "INSERT OR IGNORE INTO groups (group_id, name, description, visibility, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    group_id,
                    name,
                    description,
                    visibility.as_str(),
                    creator_pubkey,
                    created_at
                ],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        if inserted == 0 {
            return Err(SocialError::Validation(format!(
                "group '{name}' already exists"
            )));
        }

        // Auto-add the creator as owner.
        db.conn()
            .execute(
                "INSERT INTO group_members (group_id, member_pubkey, role, joined_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![group_id, creator_pubkey, "owner", created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(Group {
            group_id,
            name: name.to_string(),
            handle: None,
            description: description.map(String::from),
            avatar_cid: None,
            visibility,
            created_by: creator_pubkey.to_string(),
            created_at: Timestamp::from_secs(created_at as u64),
        })
    }

    /// Register an @handle for a group.
    ///
    /// The handle must pass the same format validation as user handles
    /// (3-20 lowercase alphanumeric + underscore, no reserved names).
    pub fn register_group_handle(
        &self,
        group_id: &str,
        handle: &str,
    ) -> Result<(), SocialError> {
        // Validate handle format (reuses user handle rules: 3-20 chars,
        // lowercase alphanumeric + underscore, no reserved names).
        validate_handle_format(handle).map_err(|e| {
            SocialError::Validation(format!("invalid group handle: {e}"))
        })?;

        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Check if the handle is already taken by another group.
        let existing: Option<String> = db
            .conn()
            .query_row(
                "SELECT group_id FROM groups WHERE handle = ?1 AND group_id != ?2",
                rusqlite::params![handle, group_id],
                |row| row.get(0),
            )
            .ok();

        if existing.is_some() {
            return Err(SocialError::Validation(format!(
                "handle '@{handle}' is already taken by another group"
            )));
        }

        db.conn()
            .execute(
                "UPDATE groups SET handle = ?1 WHERE group_id = ?2",
                rusqlite::params![handle, group_id],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Get a group by ID.
    pub fn get_group(&self, group_id: &str) -> Result<Option<Group>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let result = db
            .conn()
            .query_row(
                "SELECT group_id, name, handle, description, avatar_cid, visibility, created_by, created_at
                 FROM groups WHERE group_id = ?1",
                rusqlite::params![group_id],
                |row| {
                    Ok(Group {
                        group_id: row.get(0)?,
                        name: row.get(1)?,
                        handle: row.get(2)?,
                        description: row.get(3)?,
                        avatar_cid: row.get(4)?,
                        visibility: GroupVisibility::from_str_lossy(
                            &row.get::<_, String>(5)?,
                        ),
                        created_by: row.get(6)?,
                        created_at: Timestamp::from_secs(row.get::<_, i64>(7)? as u64),
                    })
                },
            )
            .ok();

        Ok(result)
    }

    /// Delete a group and all associated data.
    pub fn delete_group(&self, group_id: &str) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "DELETE FROM group_chat_messages WHERE chat_id IN (SELECT chat_id FROM group_chats WHERE group_id = ?1)",
                rusqlite::params![group_id],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "DELETE FROM group_chat_members WHERE chat_id IN (SELECT chat_id FROM group_chats WHERE group_id = ?1)",
                rusqlite::params![group_id],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        for table in &["group_chats", "group_posts", "group_bans", "group_members", "groups"] {
            db.conn()
                .execute(
                    &format!("DELETE FROM {table} WHERE group_id = ?1"),
                    rusqlite::params![group_id],
                )
                .map_err(|e| SocialError::Storage(e.to_string()))?;
        }

        Ok(())
    }

    // ── Membership ────────────────────────────────────────────────

    /// Join a public group (self-service).
    pub fn join_group(
        &self,
        group_id: &str,
        member_pubkey: &str,
        joined_at: i64,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Check visibility.
        let visibility: String = db
            .conn()
            .query_row(
                "SELECT visibility FROM groups WHERE group_id = ?1",
                rusqlite::params![group_id],
                |row| row.get(0),
            )
            .map_err(|_| SocialError::NotFound(format!("group {group_id} not found")))?;

        if visibility != "public" {
            return Err(SocialError::Validation(
                "cannot self-join a non-public group; an invite is required".into(),
            ));
        }

        // Check if banned.
        let is_banned: bool = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM group_bans WHERE group_id = ?1 AND banned_pubkey = ?2",
                rusqlite::params![group_id, member_pubkey],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if is_banned {
            return Err(SocialError::Validation("you are banned from this group".into()));
        }

        db.conn()
            .execute(
                "INSERT OR IGNORE INTO group_members (group_id, member_pubkey, role, joined_at)
                 VALUES (?1, ?2, 'member', ?3)",
                rusqlite::params![group_id, member_pubkey, joined_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Invite a user to a group (for private/secret groups).
    pub fn invite_to_group(
        &self,
        group_id: &str,
        target_pubkey: &str,
        invited_by: &str,
        joined_at: i64,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Check if banned.
        let is_banned: bool = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM group_bans WHERE group_id = ?1 AND banned_pubkey = ?2",
                rusqlite::params![group_id, target_pubkey],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);

        if is_banned {
            return Err(SocialError::Validation("target user is banned from this group".into()));
        }

        db.conn()
            .execute(
                "INSERT OR IGNORE INTO group_members (group_id, member_pubkey, role, joined_at, invited_by)
                 VALUES (?1, ?2, 'member', ?3, ?4)",
                rusqlite::params![group_id, target_pubkey, joined_at, invited_by],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Leave a group.
    pub fn leave_group(
        &self,
        group_id: &str,
        member_pubkey: &str,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Owners cannot leave (they must delete or transfer).
        let role: Option<String> = db
            .conn()
            .query_row(
                "SELECT role FROM group_members WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, member_pubkey],
                |row| row.get(0),
            )
            .ok();

        if role.as_deref() == Some("owner") {
            return Err(SocialError::Validation(
                "owner cannot leave; transfer ownership or delete the group".into(),
            ));
        }

        db.conn()
            .execute(
                "DELETE FROM group_members WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, member_pubkey],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Set a member's role. The caller must have a higher role.
    pub fn set_group_role(
        &self,
        group_id: &str,
        target_pubkey: &str,
        new_role: GroupRole,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "UPDATE group_members SET role = ?3 WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, target_pubkey, new_role.as_str()],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Get a member's role in a group.
    pub fn get_member_role(
        &self,
        group_id: &str,
        member_pubkey: &str,
    ) -> Result<Option<GroupRole>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let result: Option<String> = db
            .conn()
            .query_row(
                "SELECT role FROM group_members WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, member_pubkey],
                |row| row.get(0),
            )
            .ok();

        Ok(result.map(|s| GroupRole::from_str_lossy(&s)))
    }

    /// Kick a member from the group.
    pub fn kick_member(
        &self,
        group_id: &str,
        target_pubkey: &str,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "DELETE FROM group_members WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, target_pubkey],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Ban a member from the group.
    pub fn ban_member(
        &self,
        group_id: &str,
        banned_pubkey: &str,
        banned_by: &str,
        reason: Option<&str>,
        banned_at: i64,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Remove from members first.
        db.conn()
            .execute(
                "DELETE FROM group_members WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, banned_pubkey],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Add to bans.
        db.conn()
            .execute(
                "INSERT OR REPLACE INTO group_bans (group_id, banned_pubkey, banned_by, reason, banned_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![group_id, banned_pubkey, banned_by, reason, banned_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Transfer group ownership from the current owner to a target member.
    ///
    /// The current owner is demoted to admin; the target is promoted to owner.
    pub fn transfer_ownership(
        &self,
        group_id: &str,
        current_owner: &str,
        new_owner: &str,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Verify the target is a member.
        let target_role: Option<String> = db
            .conn()
            .query_row(
                "SELECT role FROM group_members WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, new_owner],
                |row| row.get(0),
            )
            .ok();

        if target_role.is_none() {
            return Err(SocialError::Validation(
                "target is not a member of this group".into(),
            ));
        }

        // Demote current owner to admin.
        db.conn()
            .execute(
                "UPDATE group_members SET role = 'admin' WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, current_owner],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        // Promote target to owner.
        db.conn()
            .execute(
                "UPDATE group_members SET role = 'owner' WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, new_owner],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Delete a post from a group (content moderation).
    pub fn delete_group_post(
        &self,
        group_id: &str,
        content_hash: &[u8],
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "DELETE FROM group_posts WHERE group_id = ?1 AND content_hash = ?2",
                rusqlite::params![group_id, content_hash],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Check if a user is a member of a group.
    pub fn is_member(&self, group_id: &str, member_pubkey: &str) -> Result<bool, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM group_members WHERE group_id = ?1 AND member_pubkey = ?2",
                rusqlite::params![group_id, member_pubkey],
                |row| row.get(0),
            )
            .unwrap_or(0);

        Ok(count > 0)
    }

    /// List members of a group.
    pub fn list_group_members(
        &self,
        group_id: &str,
    ) -> Result<Vec<GroupMember>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut stmt = db
            .conn()
            .prepare(
                "SELECT group_id, member_pubkey, role, joined_at, invited_by
                 FROM group_members WHERE group_id = ?1 ORDER BY joined_at ASC",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![group_id], |row| {
                Ok(GroupMember {
                    group_id: row.get(0)?,
                    member_pubkey: row.get(1)?,
                    role: GroupRole::from_str_lossy(&row.get::<_, String>(2)?),
                    joined_at: Timestamp::from_secs(row.get::<_, i64>(3)? as u64),
                    invited_by: row.get(4)?,
                })
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut members = Vec::new();
        for row in rows {
            members.push(row.map_err(|e| SocialError::Storage(e.to_string()))?);
        }
        Ok(members)
    }

    // ── Group feed ────────────────────────────────────────────────

    /// Post to a group (link a post content_hash to the group).
    pub fn post_to_group(
        &self,
        group_id: &str,
        content_hash: &[u8],
        author_pubkey: &str,
        created_at: i64,
    ) -> Result<(), SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        db.conn()
            .execute(
                "INSERT OR IGNORE INTO group_posts (group_id, content_hash, author_pubkey, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![group_id, content_hash, author_pubkey, created_at],
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Get a paginated feed of posts in a group.
    pub fn get_group_feed(
        &self,
        group_id: &str,
        cursor: Option<&FeedCursor>,
        limit: u32,
    ) -> Result<crate::feed::FeedPage, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;
        let fetch_limit = limit.min(200) as i64 + 1;

        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match cursor {
            Some(c) => {
                let cursor_ts = c.created_at.as_secs() as i64;
                (
                    "SELECT gp.content_hash, gp.author_pubkey, gp.created_at
                     FROM group_posts gp
                     WHERE gp.group_id = ?1
                       AND gp.created_at < ?2
                     ORDER BY gp.created_at DESC
                     LIMIT ?3",
                    vec![
                        Box::new(group_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(cursor_ts),
                        Box::new(fetch_limit),
                    ],
                )
            }
            None => (
                "SELECT gp.content_hash, gp.author_pubkey, gp.created_at
                 FROM group_posts gp
                 WHERE gp.group_id = ?1
                 ORDER BY gp.created_at DESC
                 LIMIT ?2",
                vec![
                    Box::new(group_id.to_string()) as Box<dyn rusqlite::types::ToSql>,
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
                let hash: Vec<u8> = row.get(0)?;
                let author: String = row.get(1)?;
                let created_at: i64 = row.get(2)?;
                Ok((hash, author, created_at))
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut items = Vec::new();
        for row in rows {
            let (hash, author, created_at) =
                row.map_err(|e| SocialError::Storage(e.to_string()))?;
            let content_hash = bytes_to_content_hash(&hash);
            // Parse author pubkey bytes from hex.
            let author_bytes = hex::decode(&author).unwrap_or_default();
            let author_key = bytes_to_identity_key(&author_bytes);
            items.push(crate::feed::FeedItem {
                content_hash,
                author: author_key,
                created_at: Timestamp::from_secs(created_at as u64),
                is_reply: false,
                parent: None,
            });
        }

        let has_more = items.len() > limit as usize;
        if has_more {
            items.truncate(limit as usize);
        }

        let next_cursor = if has_more {
            items.last().map(|item| FeedCursor {
                created_at: item.created_at,
                content_hash: item.content_hash.clone(),
            })
        } else {
            None
        };

        Ok(crate::feed::FeedPage {
            items,
            next_cursor,
            has_more,
        })
    }

    // ── Group queries ─────────────────────────────────────────────

    /// List groups the user is a member of.
    pub fn list_my_groups(
        &self,
        member_pubkey: &str,
    ) -> Result<Vec<GroupInfo>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut stmt = db
            .conn()
            .prepare(
                "SELECT g.group_id, g.name, g.handle, g.description, g.avatar_cid,
                        g.visibility, g.created_by, g.created_at, gm.role,
                        (SELECT COUNT(*) FROM group_members WHERE group_id = g.group_id) AS member_count
                 FROM groups g
                 JOIN group_members gm ON g.group_id = gm.group_id
                 WHERE gm.member_pubkey = ?1
                 ORDER BY g.name ASC",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![member_pubkey], |row| {
                Ok(GroupInfo {
                    group: Group {
                        group_id: row.get(0)?,
                        name: row.get(1)?,
                        handle: row.get(2)?,
                        description: row.get(3)?,
                        avatar_cid: row.get(4)?,
                        visibility: GroupVisibility::from_str_lossy(
                            &row.get::<_, String>(5)?,
                        ),
                        created_by: row.get(6)?,
                        created_at: Timestamp::from_secs(row.get::<_, i64>(7)? as u64),
                    },
                    member_count: row.get::<_, i64>(9)? as u32,
                    my_role: Some(GroupRole::from_str_lossy(&row.get::<_, String>(8)?)),
                })
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut groups = Vec::new();
        for row in rows {
            groups.push(row.map_err(|e| SocialError::Storage(e.to_string()))?);
        }
        Ok(groups)
    }

    /// Search public groups by name.
    pub fn search_groups(
        &self,
        query: &str,
    ) -> Result<Vec<GroupInfo>, SocialError> {
        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let pattern = format!("%{query}%");

        let mut stmt = db
            .conn()
            .prepare(
                "SELECT g.group_id, g.name, g.handle, g.description, g.avatar_cid,
                        g.visibility, g.created_by, g.created_at,
                        (SELECT COUNT(*) FROM group_members WHERE group_id = g.group_id) AS member_count
                 FROM groups g
                 WHERE g.visibility = 'public'
                   AND (g.name LIKE ?1 OR g.handle LIKE ?1)
                 ORDER BY member_count DESC
                 LIMIT 50",
            )
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![pattern], |row| {
                Ok(GroupInfo {
                    group: Group {
                        group_id: row.get(0)?,
                        name: row.get(1)?,
                        handle: row.get(2)?,
                        description: row.get(3)?,
                        avatar_cid: row.get(4)?,
                        visibility: GroupVisibility::from_str_lossy(
                            &row.get::<_, String>(5)?,
                        ),
                        created_by: row.get(6)?,
                        created_at: Timestamp::from_secs(row.get::<_, i64>(7)? as u64),
                    },
                    member_count: row.get::<_, i64>(8)? as u32,
                    my_role: None,
                })
            })
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let mut groups = Vec::new();
        for row in rows {
            groups.push(row.map_err(|e| SocialError::Storage(e.to_string()))?);
        }
        Ok(groups)
    }

    /// Get group info including member count and the viewer's role.
    pub fn get_group_info(
        &self,
        group_id: &str,
        viewer_pubkey: Option<&str>,
    ) -> Result<Option<GroupInfo>, SocialError> {
        let group = match self.get_group(group_id)? {
            Some(g) => g,
            None => return Ok(None),
        };

        let db = self
            .db
            .lock()
            .map_err(|e| SocialError::Storage(e.to_string()))?;

        let member_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM group_members WHERE group_id = ?1",
                rusqlite::params![group_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let my_role = if let Some(pubkey) = viewer_pubkey {
            db.conn()
                .query_row(
                    "SELECT role FROM group_members WHERE group_id = ?1 AND member_pubkey = ?2",
                    rusqlite::params![group_id, pubkey],
                    |row| row.get::<_, String>(0),
                )
                .ok()
                .map(|s| GroupRole::from_str_lossy(&s))
        } else {
            None
        };

        Ok(Some(GroupInfo {
            group,
            member_count: member_count as u32,
            my_role,
        }))
    }
}
