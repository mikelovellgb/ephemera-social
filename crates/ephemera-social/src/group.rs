//! Groups: user-created communities with roles, moderation, and handles.
//!
//! Groups evolve from topic rooms into full-featured communities with
//! ownership, role-based permissions, bans, and optional @handles.

use ephemera_types::Timestamp;
use serde::{Deserialize, Serialize};

use crate::SocialError;

/// Maximum length for a group name.
pub const MAX_GROUP_NAME_LEN: usize = 50;

/// Maximum length for a group description.
pub const MAX_GROUP_DESCRIPTION_LEN: usize = 500;

/// Visibility levels for a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GroupVisibility {
    /// Anyone can find and join.
    Public,
    /// Discoverable but requires invite or request to join.
    Private,
    /// Not discoverable; invitation only.
    Secret,
}

impl GroupVisibility {
    /// Convert to the string stored in SQLite.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Secret => "secret",
        }
    }

    /// Parse from a SQLite string.
    #[must_use]
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "private" => Self::Private,
            "secret" => Self::Secret,
            _ => Self::Public,
        }
    }
}

/// Role within a group. Ordered by privilege level (highest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum GroupRole {
    /// Full control. Can delete the group.
    Owner = 0,
    /// Can change group info, set roles below admin, ban members.
    Admin = 1,
    /// Can kick members, delete posts, invite (private groups).
    Moderator = 2,
    /// Can post and read.
    Member = 3,
}

impl GroupRole {
    /// Convert to the string stored in SQLite.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Moderator => "moderator",
            Self::Member => "member",
        }
    }

    /// Parse from a SQLite string.
    #[must_use]
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "owner" => Self::Owner,
            "admin" => Self::Admin,
            "moderator" => Self::Moderator,
            _ => Self::Member,
        }
    }

    /// Whether this role can set the target role.
    /// You can only set roles strictly below your own.
    #[must_use]
    pub fn can_set_role(&self, target: GroupRole) -> bool {
        (*self as u8) < (target as u8)
    }

    /// Whether this role can kick members.
    #[must_use]
    pub fn can_kick(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Moderator)
    }

    /// Whether this role can ban members.
    #[must_use]
    pub fn can_ban(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Whether this role can invite to private groups.
    #[must_use]
    pub fn can_invite(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Moderator)
    }

    /// Whether this role can change group info (name, description, avatar).
    #[must_use]
    pub fn can_edit_info(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Whether this role can delete any post in the group.
    #[must_use]
    pub fn can_delete_posts(&self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Moderator)
    }

    /// Whether this role can delete the group entirely.
    #[must_use]
    pub fn can_delete_group(&self) -> bool {
        matches!(self, Self::Owner)
    }
}

/// A group record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    /// BLAKE3 hash of the group name.
    pub group_id: String,
    /// Human-readable name.
    pub name: String,
    /// Optional @handle for the group.
    pub handle: Option<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Optional avatar content ID.
    pub avatar_cid: Option<String>,
    /// Visibility level.
    pub visibility: GroupVisibility,
    /// Hex-encoded pubkey of the creator.
    pub created_by: String,
    /// When the group was created (Unix seconds).
    pub created_at: Timestamp,
}

/// A group membership record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    /// The group this membership belongs to.
    pub group_id: String,
    /// Hex-encoded pubkey of the member.
    pub member_pubkey: String,
    /// Role within the group.
    pub role: GroupRole,
    /// When the member joined.
    pub joined_at: Timestamp,
    /// Who invited them (for private/secret groups).
    pub invited_by: Option<String>,
}

/// Summary info about a group (for list views).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInfo {
    /// The group record.
    pub group: Group,
    /// Number of members.
    pub member_count: u32,
    /// The requesting user's role, if a member.
    pub my_role: Option<GroupRole>,
}

/// Compute the group ID from a name.
///
/// Returns a hex-encoded BLAKE3 hash of the name.
#[must_use]
pub fn group_id_from_name(name: &str) -> String {
    let hash = blake3::hash(name.as_bytes());
    hex::encode(hash.as_bytes())
}

/// Validate a group name.
pub fn validate_group_name(name: &str) -> Result<(), SocialError> {
    if name.is_empty() {
        return Err(SocialError::Validation("group name cannot be empty".into()));
    }
    if name.chars().count() > MAX_GROUP_NAME_LEN {
        return Err(SocialError::Validation(format!(
            "group name is {} chars, max is {MAX_GROUP_NAME_LEN}",
            name.chars().count()
        )));
    }
    Ok(())
}

/// Validate a group description.
pub fn validate_group_description(desc: &str) -> Result<(), SocialError> {
    if desc.chars().count() > MAX_GROUP_DESCRIPTION_LEN {
        return Err(SocialError::Validation(format!(
            "group description is {} chars, max is {MAX_GROUP_DESCRIPTION_LEN}",
            desc.chars().count()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_id_deterministic() {
        let a = group_id_from_name("photography");
        let b = group_id_from_name("photography");
        assert_eq!(a, b);
    }

    #[test]
    fn different_names_different_ids() {
        let a = group_id_from_name("photography");
        let b = group_id_from_name("architecture");
        assert_ne!(a, b);
    }

    #[test]
    fn role_ordering() {
        assert!(GroupRole::Owner.can_set_role(GroupRole::Admin));
        assert!(GroupRole::Owner.can_set_role(GroupRole::Member));
        assert!(GroupRole::Admin.can_set_role(GroupRole::Moderator));
        assert!(!GroupRole::Admin.can_set_role(GroupRole::Owner));
        assert!(!GroupRole::Member.can_set_role(GroupRole::Member));
    }

    #[test]
    fn role_permissions() {
        assert!(GroupRole::Owner.can_delete_group());
        assert!(!GroupRole::Admin.can_delete_group());
        assert!(GroupRole::Admin.can_ban());
        assert!(!GroupRole::Moderator.can_ban());
        assert!(GroupRole::Moderator.can_kick());
        assert!(!GroupRole::Member.can_kick());
    }

    #[test]
    fn visibility_roundtrip() {
        for vis in [GroupVisibility::Public, GroupVisibility::Private, GroupVisibility::Secret] {
            assert_eq!(GroupVisibility::from_str_lossy(vis.as_str()), vis);
        }
    }

    #[test]
    fn validate_name_limits() {
        assert!(validate_group_name("").is_err());
        assert!(validate_group_name("ok").is_ok());
        assert!(validate_group_name(&"x".repeat(MAX_GROUP_NAME_LEN)).is_ok());
        assert!(validate_group_name(&"x".repeat(MAX_GROUP_NAME_LEN + 1)).is_err());
    }
}
