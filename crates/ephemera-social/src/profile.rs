//! User profile management.
//!
//! Profiles are lightweight metadata attached to a pseudonym: display name,
//! bio, and avatar. All fields are optional and size-limited.

use ephemera_types::{ContentId, IdentityKey, Timestamp};
use serde::{Deserialize, Serialize};

use crate::SocialError;

/// Maximum display name length in characters.
pub const MAX_DISPLAY_NAME_LEN: usize = 30;

/// Maximum bio length in characters.
pub const MAX_BIO_LEN: usize = 160;

/// A user's public profile metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    /// The pseudonym this profile belongs to.
    pub identity: IdentityKey,
    /// Display name (max 30 characters).
    pub display_name: Option<String>,
    /// Short bio (max 160 characters).
    pub bio: Option<String>,
    /// Content hash of the avatar image.
    pub avatar_id: Option<ContentId>,
    /// When the profile was first created.
    pub created_at: Timestamp,
    /// When the profile was last updated.
    pub updated_at: Timestamp,
}

/// Fields that can be updated on a profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileUpdate {
    /// New display name, or `None` to leave unchanged.
    pub display_name: Option<String>,
    /// New bio, or `None` to leave unchanged.
    pub bio: Option<String>,
    /// New avatar content hash, or `None` to leave unchanged.
    pub avatar_id: Option<ContentId>,
}

impl ProfileUpdate {
    /// Validate the update fields against size limits.
    pub fn validate(&self) -> Result<(), SocialError> {
        if let Some(ref name) = self.display_name {
            if name.chars().count() > MAX_DISPLAY_NAME_LEN {
                return Err(SocialError::Validation(format!(
                    "display name is {} chars, max is {MAX_DISPLAY_NAME_LEN}",
                    name.chars().count()
                )));
            }
        }
        if let Some(ref bio) = self.bio {
            if bio.chars().count() > MAX_BIO_LEN {
                return Err(SocialError::Validation(format!(
                    "bio is {} chars, max is {MAX_BIO_LEN}",
                    bio.chars().count()
                )));
            }
        }
        Ok(())
    }
}

/// Service trait for profile management.
#[async_trait::async_trait]
pub trait ProfileService: Send + Sync {
    /// Get the profile for a pseudonym, or `None` if it doesn't exist.
    async fn get(&self, identity: &IdentityKey) -> Result<Option<UserProfile>, SocialError>;

    /// Update a profile. Creates the profile if it doesn't exist.
    async fn update(
        &self,
        identity: &IdentityKey,
        update: ProfileUpdate,
    ) -> Result<UserProfile, SocialError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_profile_update() {
        let update = ProfileUpdate {
            display_name: Some("Alice".into()),
            bio: Some("Just a quiet fox.".into()),
            avatar_id: None,
        };
        assert!(update.validate().is_ok());
    }

    #[test]
    fn display_name_too_long() {
        let update = ProfileUpdate {
            display_name: Some("x".repeat(31)),
            bio: None,
            avatar_id: None,
        };
        assert!(update.validate().is_err());
    }

    #[test]
    fn bio_too_long() {
        let update = ProfileUpdate {
            display_name: None,
            bio: Some("x".repeat(161)),
            avatar_id: None,
        };
        assert!(update.validate().is_err());
    }

    #[test]
    fn empty_update_is_valid() {
        let update = ProfileUpdate::default();
        assert!(update.validate().is_ok());
    }

    #[test]
    fn boundary_lengths_are_valid() {
        let update = ProfileUpdate {
            display_name: Some("x".repeat(MAX_DISPLAY_NAME_LEN)),
            bio: Some("x".repeat(MAX_BIO_LEN)),
            avatar_id: None,
        };
        assert!(update.validate().is_ok());
    }
}
