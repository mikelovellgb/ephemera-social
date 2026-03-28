//! Human-readable handle system for Ephemera.
//!
//! Handles provide `@username`-style identifiers that map to a pseudonym's
//! Ed25519 public key. They are registered via DHT records, protected by
//! proof-of-work to deter squatting, and expire after 90 days unless renewed.
//!
//! # Design Goals
//!
//! - **Decentralized**: no central registry; handles live in the DHT.
//! - **Anti-squatting**: PoW cost scales inversely with handle length.
//! - **One handle per identity**: each pseudonym may hold at most one active handle.
//! - **Renewal required**: handles expire after 90 days if not renewed.
//! - **No transfer**: handles are bound to the registering identity.
//!
//! # Example
//!
//! ```ignore
//! let registry = HandleRegistry::new();
//! let handle = registry.register("alice", &identity, PowDifficulty::Medium)?;
//! assert_eq!(HandleRegistry::format_display(&handle), "@alice");
//! ```

use ephemera_crypto::identity::PseudonymIdentity;
use ephemera_crypto::pow::{generate_pow, PowStamp};
use ephemera_types::{IdentityKey, Signature, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result of a handle name conflict resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResult {
    /// No conflict -- the name is not taken.
    Accept,
    /// The incoming handle wins (earlier timestamp or deterministic tiebreak).
    IncomingWins,
    /// The existing handle wins (earlier timestamp or deterministic tiebreak).
    ExistingWins,
}

/// Outcome of inserting a handle received from the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    /// The handle was inserted (no prior registration or incoming won the conflict).
    Inserted,
    /// The handle was inserted and replaced an existing registration.
    /// Contains the `IdentityKey` of the displaced owner.
    Replaced { displaced_owner: IdentityKey },
    /// The incoming handle was rejected because the existing one wins.
    Rejected,
}

use crate::handle_validation::{
    build_pow_challenge, build_signature_message, calculate_pow_difficulty, validate_handle_format,
    verify_handle_pow, verify_handle_signature, HandleValidationError, PowDifficulty,
    HANDLE_REGISTRATION_COOLDOWN_SECS, HANDLE_RELEASE_COOLDOWN_SECS, HANDLE_TTL_SECS,
};

/// A registered handle record.
///
/// This is the unit of data stored in the DHT. It binds a human-readable
/// name to an Ed25519 public key, with PoW and signature proofs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handle {
    /// The handle name without the `@` prefix (e.g., `"alice"`).
    pub name: String,
    /// Ed25519 public key of the owner.
    pub owner: IdentityKey,
    /// Unix timestamp (seconds) when the handle was registered.
    pub registered_at: u64,
    /// Unix timestamp (seconds) when the handle expires.
    pub expires_at: u64,
    /// Proof-of-work stamp proving computational effort.
    pub pow_proof: PowStamp,
    /// Ed25519 signature over `(name, owner, registered_at)`.
    pub signature: Signature,
}

impl Handle {
    /// Whether this handle has expired relative to the given timestamp.
    #[must_use]
    pub fn is_expired_at(&self, now_secs: u64) -> bool {
        now_secs >= self.expires_at
    }

    /// Whether this handle has expired relative to the current wall clock.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Timestamp::now().as_secs())
    }
}

/// Errors from handle registry operations.
#[derive(Debug, thiserror::Error)]
pub enum HandleError {
    /// The handle name failed format validation.
    #[error("invalid handle format: {0}")]
    InvalidFormat(#[from] HandleValidationError),

    /// A handle with this name is already registered and has not expired.
    #[error("handle '@{name}' is already taken")]
    AlreadyTaken { name: String },

    /// The identity already owns a different active handle.
    #[error("identity already owns handle '@{existing}'; release it first")]
    IdentityAlreadyHasHandle { existing: String },

    /// Not enough time has passed since the identity's last registration.
    #[error("registration rate limited: must wait {remaining_secs}s")]
    RegistrationRateLimited { remaining_secs: u64 },

    /// The handle is in its post-release cooldown and cannot be claimed yet.
    #[error("handle '@{name}' is in cooldown until {available_at}")]
    InCooldown { name: String, available_at: u64 },

    /// The PoW proof is invalid or does not meet the required difficulty.
    #[error("invalid proof of work: {reason}")]
    InvalidPow { reason: String },

    /// The signature on the handle record is invalid.
    #[error("invalid handle signature")]
    InvalidSignature,

    /// The caller does not own this handle.
    #[error("not the owner of handle '@{name}'")]
    NotOwner { name: String },

    /// The handle has expired and cannot be renewed (must re-register).
    #[error("handle '@{name}' has expired")]
    Expired { name: String },
}

/// Local handle registry.
///
/// In production this state lives in the DHT; this struct provides a
/// local-first in-memory registry for validation, testing, and caching
/// DHT results. Nodes maintain a local view of handles they have seen
/// and validate all records cryptographically.
pub struct HandleRegistry {
    /// Active handle records, keyed by handle name.
    handles: HashMap<String, Handle>,
    /// Mapping from owner identity to their active handle name.
    owner_to_handle: HashMap<IdentityKey, String>,
    /// Timestamp of the last registration per identity (for rate limiting).
    last_registration: HashMap<IdentityKey, u64>,
    /// Handles that have been released but are still in cooldown.
    /// Maps handle name to the timestamp when cooldown ends.
    cooldowns: HashMap<String, u64>,
}

impl HandleRegistry {
    /// Mutable access to the internal handle map (for auto-renewal).
    pub fn handles_mut(&mut self) -> &mut HashMap<String, Handle> {
        &mut self.handles
    }

    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handles: HashMap::new(),
            owner_to_handle: HashMap::new(),
            last_registration: HashMap::new(),
            cooldowns: HashMap::new(),
        }
    }

    /// Register a new handle for the given identity.
    ///
    /// This performs the full PoW computation, which may take seconds to
    /// minutes depending on the handle length. The difficulty is
    /// determined automatically from the handle name length.
    ///
    /// # Errors
    ///
    /// Returns a [`HandleError`] if the handle name is invalid, already
    /// taken, or the identity already owns a handle.
    pub fn register(
        &mut self,
        name: &str,
        identity: &PseudonymIdentity,
        now_secs: u64,
    ) -> Result<Handle, HandleError> {
        let normalized = name.to_ascii_lowercase();
        let difficulty = calculate_pow_difficulty(&normalized);
        self.register_with_difficulty(&normalized, identity, difficulty, now_secs)
    }

    /// Register a handle with an explicit difficulty (useful for testing
    /// and renewals).
    pub fn register_with_difficulty(
        &mut self,
        name: &str,
        identity: &PseudonymIdentity,
        difficulty: PowDifficulty,
        now_secs: u64,
    ) -> Result<Handle, HandleError> {
        // 1. Validate format
        validate_handle_format(name)?;

        let owner = identity_key_from_pseudonym(identity);

        // 2. Check one-handle-per-identity
        if let Some(existing) = self.owner_to_handle.get(&owner) {
            // Check if the existing handle is still active
            if let Some(h) = self.handles.get(existing) {
                if !h.is_expired_at(now_secs) {
                    return Err(HandleError::IdentityAlreadyHasHandle {
                        existing: existing.clone(),
                    });
                }
                // Existing handle expired; clean it up
                let old_name = existing.clone();
                self.handles.remove(&old_name);
                self.owner_to_handle.remove(&owner);
            }
        }

        // 3. Rate limiting
        if let Some(&last) = self.last_registration.get(&owner) {
            let elapsed = now_secs.saturating_sub(last);
            if elapsed < HANDLE_REGISTRATION_COOLDOWN_SECS {
                return Err(HandleError::RegistrationRateLimited {
                    remaining_secs: HANDLE_REGISTRATION_COOLDOWN_SECS - elapsed,
                });
            }
        }

        // 4. Check handle availability
        if let Some(existing) = self.handles.get(name) {
            if !existing.is_expired_at(now_secs) {
                return Err(HandleError::AlreadyTaken {
                    name: name.to_string(),
                });
            }
            // Expired: remove it
            let old_owner = existing.owner;
            self.handles.remove(name);
            self.owner_to_handle.remove(&old_owner);
        }

        // 5. Check cooldown
        if let Some(&cooldown_until) = self.cooldowns.get(name) {
            if now_secs < cooldown_until {
                return Err(HandleError::InCooldown {
                    name: name.to_string(),
                    available_at: cooldown_until,
                });
            }
            // Cooldown passed; remove entry
            self.cooldowns.remove(name);
        }

        // 6. Compute PoW
        let challenge = build_pow_challenge(name, &owner, now_secs);
        let pow_proof = generate_pow(&challenge, difficulty.bits());

        // 7. Sign
        let sig_msg = build_signature_message(name, &owner, now_secs);
        let sig_bytes = identity
            .sign(&sig_msg)
            .map_err(|_| HandleError::InvalidSignature)?;
        let signature = Signature::from_slice(&sig_bytes).ok_or(HandleError::InvalidSignature)?;

        // 8. Build record
        let handle = Handle {
            name: name.to_string(),
            owner,
            registered_at: now_secs,
            expires_at: now_secs + HANDLE_TTL_SECS,
            pow_proof,
            signature,
        };

        // 9. Store locally
        self.handles.insert(name.to_string(), handle.clone());
        self.owner_to_handle.insert(owner, name.to_string());
        self.last_registration.insert(owner, now_secs);

        Ok(handle)
    }

    /// Validate a handle record received from the network (e.g., from DHT).
    ///
    /// Checks format, signature, PoW, and expiry. Does NOT check local
    /// state (collisions, rate limits) -- those are checked on insertion.
    ///
    /// # Errors
    ///
    /// Returns a [`HandleError`] if any validation check fails.
    pub fn validate(handle: &Handle, now_secs: u64) -> Result<(), HandleError> {
        // 1. Format
        validate_handle_format(&handle.name)?;

        // 2. Expiry
        if handle.is_expired_at(now_secs) {
            return Err(HandleError::Expired {
                name: handle.name.clone(),
            });
        }

        // 3. TTL sanity: expires_at should be registered_at + HANDLE_TTL_SECS
        if handle.expires_at != handle.registered_at + HANDLE_TTL_SECS {
            return Err(HandleError::InvalidPow {
                reason: "expires_at does not match registered_at + TTL".into(),
            });
        }

        // 4. Signature
        verify_handle_signature(
            &handle.name,
            &handle.owner,
            handle.registered_at,
            &handle.signature,
        )
        .map_err(|_| HandleError::InvalidSignature)?;

        // 5. PoW
        let required_difficulty = calculate_pow_difficulty(&handle.name);
        verify_handle_pow(
            &handle.name,
            &handle.owner,
            handle.registered_at,
            &handle.pow_proof,
            required_difficulty,
        )
        .map_err(|e| HandleError::InvalidPow {
            reason: e.to_string(),
        })?;

        Ok(())
    }

    /// Look up a handle by name.
    ///
    /// In production, this would issue a DHT query. The local registry
    /// serves as a cache of previously-seen handles.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&Handle> {
        self.handles.get(&name.to_ascii_lowercase())
    }

    /// Look up a handle by owner identity (reverse lookup).
    #[must_use]
    pub fn lookup_by_owner(&self, owner: &IdentityKey) -> Option<&Handle> {
        self.owner_to_handle
            .get(owner)
            .and_then(|name| self.handles.get(name))
    }

    /// Renew an existing handle, extending its expiry by another 90 days.
    ///
    /// Renewal requires a PoW proof at [`PowDifficulty::Renewal`] difficulty
    /// (lower than initial registration).
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::NotOwner`] if the identity does not own the
    /// handle, or [`HandleError::Expired`] if the handle has already expired.
    pub fn renew(
        &mut self,
        name: &str,
        identity: &PseudonymIdentity,
        now_secs: u64,
    ) -> Result<(), HandleError> {
        let owner = identity_key_from_pseudonym(identity);

        let handle = self.handles.get(name).ok_or_else(|| HandleError::Expired {
            name: name.to_string(),
        })?;

        // Must be the owner
        if handle.owner != owner {
            return Err(HandleError::NotOwner {
                name: name.to_string(),
            });
        }

        // Must not be expired
        if handle.is_expired_at(now_secs) {
            return Err(HandleError::Expired {
                name: name.to_string(),
            });
        }

        // Compute renewal PoW
        let challenge = build_pow_challenge(name, &owner, now_secs);
        let pow_proof = generate_pow(&challenge, PowDifficulty::Renewal.bits());

        // Sign the renewal
        let sig_msg = build_signature_message(name, &owner, now_secs);
        let sig_bytes = identity
            .sign(&sig_msg)
            .map_err(|_| HandleError::InvalidSignature)?;
        let signature = Signature::from_slice(&sig_bytes).ok_or(HandleError::InvalidSignature)?;

        // Update the record in place
        let handle_mut = self.handles.get_mut(name).expect("just checked existence");
        handle_mut.registered_at = now_secs;
        handle_mut.expires_at = now_secs + HANDLE_TTL_SECS;
        handle_mut.pow_proof = pow_proof;
        handle_mut.signature = signature;

        Ok(())
    }

    /// Release a handle, making it available for others after a 24-hour
    /// cooldown.
    ///
    /// # Errors
    ///
    /// Returns [`HandleError::NotOwner`] if the identity does not own the
    /// handle.
    pub fn release(
        &mut self,
        name: &str,
        identity: &PseudonymIdentity,
        now_secs: u64,
    ) -> Result<(), HandleError> {
        let owner = identity_key_from_pseudonym(identity);

        let handle = self
            .handles
            .get(name)
            .ok_or_else(|| HandleError::NotOwner {
                name: name.to_string(),
            })?;

        if handle.owner != owner {
            return Err(HandleError::NotOwner {
                name: name.to_string(),
            });
        }

        // Remove from active handles
        self.handles.remove(name);
        self.owner_to_handle.remove(&owner);

        // Set cooldown
        self.cooldowns
            .insert(name.to_string(), now_secs + HANDLE_RELEASE_COOLDOWN_SECS);

        Ok(())
    }

    /// Format a handle for display with the `@` prefix.
    #[must_use]
    pub fn format_display(handle: &Handle) -> String {
        format!("@{}", handle.name)
    }

    /// Determine whether an incoming handle should replace an existing one.
    ///
    /// Resolution rules:
    /// 1. If no existing handle with that name, accept.
    /// 2. If existing handle is expired, accept.
    /// 3. Earlier `registered_at` timestamp wins.
    /// 4. If timestamps are equal, the lower pubkey bytes win (deterministic).
    pub fn resolve_conflict(&self, incoming: &Handle, now_secs: u64) -> ConflictResult {
        match self.handles.get(&incoming.name) {
            None => ConflictResult::Accept,
            Some(existing) => {
                // Expired handles do not block incoming registrations.
                if existing.is_expired_at(now_secs) {
                    return ConflictResult::Accept;
                }

                if incoming.registered_at < existing.registered_at {
                    ConflictResult::IncomingWins
                } else if incoming.registered_at == existing.registered_at {
                    // Deterministic tiebreak: lower pubkey bytes win.
                    if incoming.owner.as_bytes() < existing.owner.as_bytes() {
                        ConflictResult::IncomingWins
                    } else {
                        ConflictResult::ExistingWins
                    }
                } else {
                    ConflictResult::ExistingWins
                }
            }
        }
    }

    /// Insert a pre-validated handle into the local registry.
    ///
    /// Used when receiving handle records from the DHT. The caller is
    /// responsible for calling [`Self::validate`] first.
    ///
    /// Returns an [`InsertOutcome`] describing what happened:
    /// - `Inserted` if the name was free (or existing was expired).
    /// - `Replaced { displaced_owner }` if the incoming handle won a conflict.
    /// - `Rejected` if the existing handle wins the conflict.
    ///
    /// Legacy callers that only need a bool can use
    /// `outcome != InsertOutcome::Rejected`.
    pub fn insert_validated(&mut self, handle: Handle, now_secs: u64) -> InsertOutcome {
        let conflict = self.resolve_conflict(&handle, now_secs);

        match conflict {
            ConflictResult::ExistingWins => InsertOutcome::Rejected,
            ConflictResult::Accept => {
                // Remove any previous (expired) owner mapping for this name.
                if let Some(prev) = self.handles.get(&handle.name) {
                    self.owner_to_handle.remove(&prev.owner);
                }

                let name = handle.name.clone();
                let owner = handle.owner;
                self.handles.insert(name.clone(), handle);
                self.owner_to_handle.insert(owner, name);
                InsertOutcome::Inserted
            }
            ConflictResult::IncomingWins => {
                // The existing registration loses. Record the displaced owner
                // so the caller can notify them.
                let displaced_owner = self
                    .handles
                    .get(&handle.name)
                    .map(|h| h.owner)
                    .expect("IncomingWins implies existing handle");

                // Remove the old owner's mapping.
                self.owner_to_handle.remove(&displaced_owner);

                let name = handle.name.clone();
                let owner = handle.owner;
                self.handles.insert(name.clone(), handle);
                self.owner_to_handle.insert(owner, name);
                InsertOutcome::Replaced { displaced_owner }
            }
        }
    }

    /// Remove expired handles and completed cooldowns from the registry.
    ///
    /// Call this periodically (e.g., once per minute) to keep the local
    /// state clean.
    /// Search for handles whose name starts with or contains the given query.
    pub fn search_prefix(&self, query: &str) -> Vec<&Handle> {
        let q = query.to_lowercase();
        self.handles
            .values()
            .filter(|h| h.name.to_lowercase().contains(&q))
            .collect()
    }

    pub fn gc(&mut self, now_secs: u64) {
        let expired_names: Vec<String> = self
            .handles
            .iter()
            .filter(|(_, h)| h.is_expired_at(now_secs))
            .map(|(name, _)| name.clone())
            .collect();

        for name in expired_names {
            if let Some(handle) = self.handles.remove(&name) {
                self.owner_to_handle.remove(&handle.owner);
            }
        }

        self.cooldowns.retain(|_, &mut until| now_secs < until);
    }
}

impl Default for HandleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract an `IdentityKey` from a `PseudonymIdentity`.
fn identity_key_from_pseudonym(identity: &PseudonymIdentity) -> IdentityKey {
    IdentityKey::from_bytes(*identity.identity_key().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemera_crypto::identity::PseudonymIdentity;
    use ephemera_crypto::keys::MasterSecret;

    /// Helper to create a test identity at a given index.
    fn test_identity(index: u32) -> PseudonymIdentity {
        let master = MasterSecret::from_bytes([index as u8; 32]);
        PseudonymIdentity::derive(&master, index).unwrap()
    }

    /// A base timestamp for tests (2024-01-01 00:00:00 UTC).
    const BASE_TS: u64 = 1_704_067_200;

    // ── Registration ─────────────────────────────────────────────────

    #[test]
    fn test_register_valid_handle() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        // Use a long handle so difficulty is low and the test is fast
        let handle = registry
            .register_with_difficulty("long_handle_name", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();
        assert_eq!(handle.name, "long_handle_name");
        assert_eq!(handle.registered_at, BASE_TS);
        assert_eq!(handle.expires_at, BASE_TS + HANDLE_TTL_SECS);
        assert_eq!(HandleRegistry::format_display(&handle), "@long_handle_name");
    }

    #[test]
    fn test_register_invalid_format() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        let result = registry.register_with_difficulty("ab", &id, PowDifficulty::Low, BASE_TS);
        assert!(matches!(result, Err(HandleError::InvalidFormat(_))));
    }

    #[test]
    fn test_one_handle_per_identity() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        registry
            .register_with_difficulty("first_handle", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let result = registry.register_with_difficulty(
            "second_handle",
            &id,
            PowDifficulty::Low,
            BASE_TS + HANDLE_REGISTRATION_COOLDOWN_SECS,
        );
        assert!(matches!(
            result,
            Err(HandleError::IdentityAlreadyHasHandle { .. })
        ));
    }

    #[test]
    fn test_handle_already_taken() {
        let mut registry = HandleRegistry::new();
        let id1 = test_identity(0);
        let id2 = test_identity(1);
        registry
            .register_with_difficulty("taken_handle", &id1, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let result = registry.register_with_difficulty(
            "taken_handle",
            &id2,
            PowDifficulty::Low,
            BASE_TS + HANDLE_REGISTRATION_COOLDOWN_SECS,
        );
        assert!(matches!(result, Err(HandleError::AlreadyTaken { .. })));
    }

    #[test]
    fn test_registration_rate_limit() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        // First, register and release so the identity has no active handle
        // but has a last_registration timestamp
        registry
            .register_with_difficulty("temp_handle", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();
        registry.release("temp_handle", &id, BASE_TS + 1).unwrap();

        // Try to register again too soon (within 24h cooldown)
        let result = registry.register_with_difficulty(
            "new_handle_name",
            &id,
            PowDifficulty::Low,
            BASE_TS + 100,
        );
        assert!(matches!(
            result,
            Err(HandleError::RegistrationRateLimited { .. })
        ));
    }

    // ── Lookup ───────────────────────────────────────────────────────

    #[test]
    fn test_lookup_by_name() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        registry
            .register_with_difficulty("lookup_test", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let result = registry.lookup("lookup_test");
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "lookup_test");
    }

    #[test]
    fn test_lookup_by_owner() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        let owner = identity_key_from_pseudonym(&id);
        registry
            .register_with_difficulty("owner_lookup", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let result = registry.lookup_by_owner(&owner);
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "owner_lookup");
    }

    #[test]
    fn test_lookup_nonexistent() {
        let registry = HandleRegistry::new();
        assert!(registry.lookup("nonexistent").is_none());
    }

    // ── Expiry ───────────────────────────────────────────────────────

    #[test]
    fn test_handle_expiry() {
        let mut registry = HandleRegistry::new();
        let id1 = test_identity(0);
        let id2 = test_identity(1);
        registry
            .register_with_difficulty("expiring", &id1, PowDifficulty::Low, BASE_TS)
            .unwrap();

        // Handle should be taken
        assert!(registry
            .register_with_difficulty("expiring", &id2, PowDifficulty::Low, BASE_TS + 100)
            .is_err());

        // After 90 days, the handle expires and can be reclaimed
        let after_expiry = BASE_TS + HANDLE_TTL_SECS + 1;
        let result =
            registry.register_with_difficulty("expiring", &id2, PowDifficulty::Low, after_expiry);
        assert!(result.is_ok());
    }

    #[test]
    fn test_expired_own_handle_allows_new_registration() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        registry
            .register_with_difficulty("old_handle", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        // After expiry + rate limit, the same identity can register a new handle
        let after_expiry = BASE_TS + HANDLE_TTL_SECS + HANDLE_REGISTRATION_COOLDOWN_SECS + 1;
        let result =
            registry.register_with_difficulty("new_handle", &id, PowDifficulty::Low, after_expiry);
        assert!(result.is_ok());
    }

    // ── Renewal ──────────────────────────────────────────────────────

    #[test]
    fn test_handle_renewal() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        registry
            .register_with_difficulty("renewable", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        // Renew before expiry
        let renew_time = BASE_TS + (HANDLE_TTL_SECS / 2);
        let result = registry.renew("renewable", &id, renew_time);
        assert!(result.is_ok());

        // Check that expiry was extended
        let handle = registry.lookup("renewable").unwrap();
        assert_eq!(handle.expires_at, renew_time + HANDLE_TTL_SECS);
    }

    #[test]
    fn test_renewal_by_non_owner_fails() {
        let mut registry = HandleRegistry::new();
        let id1 = test_identity(0);
        let id2 = test_identity(1);
        registry
            .register_with_difficulty("owned_handle", &id1, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let result = registry.renew("owned_handle", &id2, BASE_TS + 1000);
        assert!(matches!(result, Err(HandleError::NotOwner { .. })));
    }

    #[test]
    fn test_renewal_of_expired_handle_fails() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        registry
            .register_with_difficulty("will_expire", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let after_expiry = BASE_TS + HANDLE_TTL_SECS + 1;
        let result = registry.renew("will_expire", &id, after_expiry);
        assert!(matches!(result, Err(HandleError::Expired { .. })));
    }

    // ── Release ──────────────────────────────────────────────────────

    #[test]
    fn test_release_handle() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        registry
            .register_with_difficulty("releasable", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let result = registry.release("releasable", &id, BASE_TS + 1);
        assert!(result.is_ok());

        // Handle should no longer be active
        assert!(registry.lookup("releasable").is_none());
    }

    #[test]
    fn test_release_by_non_owner_fails() {
        let mut registry = HandleRegistry::new();
        let id1 = test_identity(0);
        let id2 = test_identity(1);
        registry
            .register_with_difficulty("not_yours", &id1, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let result = registry.release("not_yours", &id2, BASE_TS + 1);
        assert!(matches!(result, Err(HandleError::NotOwner { .. })));
    }

    #[test]
    fn test_release_cooldown() {
        let mut registry = HandleRegistry::new();
        let id1 = test_identity(0);
        let id2 = test_identity(1);
        registry
            .register_with_difficulty("cooldown_test", &id1, PowDifficulty::Low, BASE_TS)
            .unwrap();
        registry
            .release("cooldown_test", &id1, BASE_TS + 1)
            .unwrap();

        // Another identity tries to claim during cooldown
        let during_cooldown = BASE_TS + 100;
        let result = registry.register_with_difficulty(
            "cooldown_test",
            &id2,
            PowDifficulty::Low,
            during_cooldown,
        );
        assert!(matches!(result, Err(HandleError::InCooldown { .. })));

        // After cooldown passes
        let after_cooldown = BASE_TS + 1 + HANDLE_RELEASE_COOLDOWN_SECS + 1;
        let result = registry.register_with_difficulty(
            "cooldown_test",
            &id2,
            PowDifficulty::Low,
            after_cooldown,
        );
        assert!(result.is_ok());
    }

    // ── Validation ───────────────────────────────────────────────────

    #[test]
    fn test_validate_handle_record() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        let handle = registry
            .register_with_difficulty("valid_record", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        // Full record validation should pass
        assert!(HandleRegistry::validate(&handle, BASE_TS).is_ok());
    }

    #[test]
    fn test_validate_expired_handle_fails() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        let handle = registry
            .register_with_difficulty("will_expire_v", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        let after_expiry = BASE_TS + HANDLE_TTL_SECS + 1;
        assert!(matches!(
            HandleRegistry::validate(&handle, after_expiry),
            Err(HandleError::Expired { .. })
        ));
    }

    #[test]
    fn test_validate_tampered_signature_fails() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        let mut handle = registry
            .register_with_difficulty("tamper_test", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        // Tamper with the owner
        handle.owner = IdentityKey::from_bytes([0xFF; 32]);

        let result = HandleRegistry::validate(&handle, BASE_TS);
        assert!(
            matches!(result, Err(HandleError::InvalidSignature))
                || matches!(result, Err(HandleError::InvalidPow { .. }))
        );
    }

    // ── GC ───────────────────────────────────────────────────────────

    #[test]
    fn test_gc_removes_expired() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        registry
            .register_with_difficulty("gc_test", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();

        assert!(registry.lookup("gc_test").is_some());

        let after_expiry = BASE_TS + HANDLE_TTL_SECS + 1;
        registry.gc(after_expiry);

        assert!(registry.lookup("gc_test").is_none());
    }

    #[test]
    fn test_gc_removes_completed_cooldowns() {
        let mut registry = HandleRegistry::new();
        let id = test_identity(0);
        registry
            .register_with_difficulty("gc_cd_test", &id, PowDifficulty::Low, BASE_TS)
            .unwrap();
        registry.release("gc_cd_test", &id, BASE_TS + 1).unwrap();

        // Cooldown should exist
        assert!(registry.cooldowns.contains_key("gc_cd_test"));

        // After cooldown
        let after_cooldown = BASE_TS + 1 + HANDLE_RELEASE_COOLDOWN_SECS + 1;
        registry.gc(after_cooldown);

        assert!(!registry.cooldowns.contains_key("gc_cd_test"));
    }

    // ── Insert validated (DHT simulation) ────────────────────────────

    #[test]
    fn test_insert_validated_first_wins() {
        let mut registry = HandleRegistry::new();
        let id1 = test_identity(0);
        let id2 = test_identity(1);

        // Register handle for id1
        let handle1 = registry
            .register_with_difficulty("contested", &id1, PowDifficulty::Low, BASE_TS)
            .unwrap();

        // Simulate receiving a later registration from id2 via DHT
        let owner2 = identity_key_from_pseudonym(&id2);
        let challenge = build_pow_challenge("contested", &owner2, BASE_TS + 100);
        let pow_proof = generate_pow(&challenge, PowDifficulty::Low.bits());
        let sig_msg = build_signature_message("contested", &owner2, BASE_TS + 100);
        let sig_bytes = id2.sign(&sig_msg).unwrap();
        let signature = Signature::from_slice(&sig_bytes).unwrap();

        let handle2 = Handle {
            name: "contested".into(),
            owner: owner2,
            registered_at: BASE_TS + 100,
            expires_at: BASE_TS + 100 + HANDLE_TTL_SECS,
            pow_proof,
            signature,
        };

        // Should be rejected: first registration (earlier timestamp) wins
        let outcome = registry.insert_validated(handle2, BASE_TS + 100);
        assert_eq!(outcome, InsertOutcome::Rejected);

        // Original owner should still hold the handle
        let current = registry.lookup("contested").unwrap();
        assert_eq!(current.owner, handle1.owner);
    }

    // ── Conflict resolution ─────────────────────────────────────────

    /// Helper: build a Handle record for the given identity at a given timestamp.
    fn make_handle(name: &str, identity: &PseudonymIdentity, ts: u64) -> Handle {
        let owner = identity_key_from_pseudonym(identity);
        let challenge = build_pow_challenge(name, &owner, ts);
        let pow_proof = generate_pow(&challenge, PowDifficulty::Low.bits());
        let sig_msg = build_signature_message(name, &owner, ts);
        let sig_bytes = identity.sign(&sig_msg).unwrap();
        let signature = Signature::from_slice(&sig_bytes).unwrap();
        Handle {
            name: name.to_string(),
            owner,
            registered_at: ts,
            expires_at: ts + HANDLE_TTL_SECS,
            pow_proof,
            signature,
        }
    }

    #[test]
    fn test_conflict_earliest_wins() {
        let mut registry = HandleRegistry::new();
        let id_early = test_identity(10);
        let id_late = test_identity(11);

        // id_late registers first locally (at a later timestamp).
        let late_handle = make_handle("conflict_name", &id_late, BASE_TS + 500);
        let outcome = registry.insert_validated(late_handle, BASE_TS + 500);
        assert_eq!(outcome, InsertOutcome::Inserted);
        assert_eq!(
            registry.lookup("conflict_name").unwrap().owner,
            identity_key_from_pseudonym(&id_late),
        );

        // id_early arrives via gossip with an earlier timestamp -- should win.
        let early_handle = make_handle("conflict_name", &id_early, BASE_TS + 100);
        let outcome = registry.insert_validated(early_handle, BASE_TS + 600);
        assert!(matches!(outcome, InsertOutcome::Replaced { .. }));
        assert_eq!(
            registry.lookup("conflict_name").unwrap().owner,
            identity_key_from_pseudonym(&id_early),
        );
    }

    #[test]
    fn test_conflict_tiebreak_by_pubkey() {
        let id_a = test_identity(20);
        let id_b = test_identity(21);
        let owner_a = identity_key_from_pseudonym(&id_a);
        let owner_b = identity_key_from_pseudonym(&id_b);

        // Determine which pubkey is lower so we know the expected winner.
        let (winner_id, loser_id) = if owner_a.as_bytes() < owner_b.as_bytes() {
            (&id_a, &id_b)
        } else {
            (&id_b, &id_a)
        };
        let winner_owner = identity_key_from_pseudonym(winner_id);

        // Both register at the exact same timestamp.
        let same_ts = BASE_TS + 200;

        let mut registry = HandleRegistry::new();
        let loser_handle = make_handle("tiebreak_nm", loser_id, same_ts);
        let outcome = registry.insert_validated(loser_handle, same_ts);
        assert_eq!(outcome, InsertOutcome::Inserted);

        let winner_handle = make_handle("tiebreak_nm", winner_id, same_ts);
        let outcome = registry.insert_validated(winner_handle, same_ts);
        assert!(matches!(outcome, InsertOutcome::Replaced { .. }));

        // The winner (lower pubkey) should now own the handle.
        assert_eq!(registry.lookup("tiebreak_nm").unwrap().owner, winner_owner);
    }

    #[test]
    fn test_loser_handle_removed() {
        let mut registry = HandleRegistry::new();
        let id_early = test_identity(30);
        let id_late = test_identity(31);
        let owner_late = identity_key_from_pseudonym(&id_late);

        // id_late registers first locally.
        let late_handle = make_handle("remove_test", &id_late, BASE_TS + 500);
        registry.insert_validated(late_handle, BASE_TS + 500);

        // Verify the late owner's reverse lookup works.
        assert!(registry.lookup_by_owner(&owner_late).is_some());

        // id_early arrives and wins the conflict.
        let early_handle = make_handle("remove_test", &id_early, BASE_TS + 100);
        let outcome = registry.insert_validated(early_handle, BASE_TS + 600);
        assert_eq!(
            outcome,
            InsertOutcome::Replaced {
                displaced_owner: owner_late,
            },
        );

        // The loser's reverse lookup should no longer return anything.
        assert!(registry.lookup_by_owner(&owner_late).is_none());

        // The handle should belong to the winner.
        assert_eq!(
            registry.lookup("remove_test").unwrap().owner,
            identity_key_from_pseudonym(&id_early),
        );
    }

    // ── Format display ───────────────────────────────────────────────

    #[test]
    fn test_format_display() {
        let handle = Handle {
            name: "alice".into(),
            owner: IdentityKey::from_bytes([0; 32]),
            registered_at: BASE_TS,
            expires_at: BASE_TS + HANDLE_TTL_SECS,
            pow_proof: PowStamp {
                challenge: vec![],
                nonce: [0; 16],
                difficulty: 0,
            },
            signature: Signature::from_bytes([0; 64]),
        };
        assert_eq!(HandleRegistry::format_display(&handle), "@alice");
    }
}
