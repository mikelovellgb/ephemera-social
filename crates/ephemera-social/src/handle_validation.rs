//! Handle format validation, PoW difficulty calculation, and cryptographic
//! verification for the Ephemera handle system.
//!
//! Handles are human-readable `@username` identifiers that map to a
//! pseudonym's Ed25519 public key via DHT records. This module provides
//! the pure validation logic; the registry (state management) lives in
//! [`super::handle`].

use ephemera_crypto::pow::{self, PowStamp};
use ephemera_crypto::signing;
use ephemera_types::IdentityKey;

/// Minimum handle length (inclusive).
pub const MIN_HANDLE_LEN: usize = 3;

/// Maximum handle length (inclusive).
pub const MAX_HANDLE_LEN: usize = 20;

/// PoW difficulty for short handles (3-5 characters).
///
/// With Argon2id (~200ms per attempt), 2^11 expected attempts ≈ **7 minutes**.
/// Premium short handles should be expensive to claim.
pub const POW_DIFFICULTY_HIGH: u32 = 11;

/// PoW difficulty for medium handles (6-10 characters).
///
/// With Argon2id (~200ms per attempt), 2^9 expected attempts ≈ **100 seconds**.
pub const POW_DIFFICULTY_MEDIUM: u32 = 9;

/// PoW difficulty for long handles (11-20 characters).
///
/// With Argon2id (~200ms per attempt), 2^7 expected attempts ≈ **25 seconds**.
pub const POW_DIFFICULTY_LOW: u32 = 7;

/// PoW difficulty for handle renewals (lower than initial registration).
///
/// With Argon2id (~200ms per attempt), 2^5 expected attempts ≈ **6 seconds**.
pub const POW_DIFFICULTY_RENEWAL: u32 = 5;

/// Handle TTL in seconds (90 days).
pub const HANDLE_TTL_SECS: u64 = 90 * 24 * 60 * 60;

/// Cooldown period in seconds after releasing a handle before it can be
/// reclaimed by anyone (24 hours).
pub const HANDLE_RELEASE_COOLDOWN_SECS: u64 = 24 * 60 * 60;

/// Rate limit: minimum seconds between handle registrations for the same
/// identity (24 hours).
pub const HANDLE_REGISTRATION_COOLDOWN_SECS: u64 = 24 * 60 * 60;

/// Domain separator used when constructing the PoW challenge for a handle.
const POW_DOMAIN: &[u8] = b"ephemera-handle-pow-v1\x00";

/// Domain separator used when constructing the signature message for a handle.
const SIG_DOMAIN: &[u8] = b"ephemera-handle-sig-v1\x00";

/// Reserved handle names that cannot be registered by any user.
const RESERVED_HANDLES: &[&str] = &[
    "admin",
    "administrator",
    "ephemera",
    "help",
    "info",
    "mod",
    "moderator",
    "official",
    "root",
    "staff",
    "support",
    "system",
    "team",
    "trust",
    "security",
];

/// Proof-of-work difficulty tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowDifficulty {
    /// Short handles (3-5 chars): ~5 minutes.
    High,
    /// Medium handles (6-10 chars): ~30 seconds.
    Medium,
    /// Long handles (11-20 chars): ~5 seconds.
    Low,
    /// Renewal difficulty: ~1 second.
    Renewal,
}

impl PowDifficulty {
    /// The number of leading zero bits required for this difficulty tier.
    #[must_use]
    pub fn bits(self) -> u32 {
        match self {
            Self::High => POW_DIFFICULTY_HIGH,
            Self::Medium => POW_DIFFICULTY_MEDIUM,
            Self::Low => POW_DIFFICULTY_LOW,
            Self::Renewal => POW_DIFFICULTY_RENEWAL,
        }
    }
}

/// Errors specific to handle validation.
#[derive(Debug, thiserror::Error)]
pub enum HandleValidationError {
    /// The handle name is too short.
    #[error("handle too short: {len} chars, minimum is {MIN_HANDLE_LEN}")]
    TooShort { len: usize },

    /// The handle name is too long.
    #[error("handle too long: {len} chars, maximum is {MAX_HANDLE_LEN}")]
    TooLong { len: usize },

    /// The handle contains invalid characters.
    #[error("handle contains invalid character '{ch}' at position {pos}: only lowercase a-z, 0-9, and underscores are allowed")]
    InvalidCharacter { ch: char, pos: usize },

    /// The handle starts with an underscore.
    #[error("handle must not start with an underscore")]
    StartsWithUnderscore,

    /// The handle ends with an underscore.
    #[error("handle must not end with an underscore")]
    EndsWithUnderscore,

    /// The handle contains consecutive underscores.
    #[error("handle must not contain consecutive underscores")]
    ConsecutiveUnderscores,

    /// The handle is a reserved name.
    #[error("handle '{name}' is reserved")]
    Reserved { name: String },

    /// The handle starts with a numeric character.
    #[error("handle must not start with a digit")]
    StartsWithDigit,
}

/// Validate a handle name against format rules.
///
/// Valid handles are:
/// - 3-20 characters long
/// - Lowercase ASCII letters, digits, and underscores only
/// - Must start with a letter (not digit or underscore)
/// - Must not end with an underscore
/// - Must not contain consecutive underscores
/// - Must not be a reserved name
///
/// # Errors
///
/// Returns a [`HandleValidationError`] describing the first rule violation.
pub fn validate_handle_format(name: &str) -> Result<(), HandleValidationError> {
    let len = name.len();

    // Length checks
    if len < MIN_HANDLE_LEN {
        return Err(HandleValidationError::TooShort { len });
    }
    if len > MAX_HANDLE_LEN {
        return Err(HandleValidationError::TooLong { len });
    }

    // Character validation
    for (pos, ch) in name.chars().enumerate() {
        if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '_' {
            return Err(HandleValidationError::InvalidCharacter { ch, pos });
        }
    }

    // Structural rules
    if name.starts_with('_') {
        return Err(HandleValidationError::StartsWithUnderscore);
    }
    if name.ends_with('_') {
        return Err(HandleValidationError::EndsWithUnderscore);
    }
    if name.contains("__") {
        return Err(HandleValidationError::ConsecutiveUnderscores);
    }

    // First character must be a letter, not a digit
    if name.as_bytes()[0].is_ascii_digit() {
        return Err(HandleValidationError::StartsWithDigit);
    }

    // Reserved names
    if RESERVED_HANDLES.contains(&name) {
        return Err(HandleValidationError::Reserved {
            name: name.to_string(),
        });
    }

    Ok(())
}

/// Determine the PoW difficulty tier for a handle based on its length.
///
/// - 3-5 characters: [`PowDifficulty::High`] (~5 minutes)
/// - 6-10 characters: [`PowDifficulty::Medium`] (~30 seconds)
/// - 11-20 characters: [`PowDifficulty::Low`] (~5 seconds)
#[must_use]
pub fn calculate_pow_difficulty(name: &str) -> PowDifficulty {
    let len = name.len();
    if len <= 5 {
        PowDifficulty::High
    } else if len <= 10 {
        PowDifficulty::Medium
    } else {
        PowDifficulty::Low
    }
}

/// Build the PoW challenge bytes for a handle registration.
///
/// The challenge binds the handle name, owner pubkey, and registration
/// timestamp together so that a PoW proof cannot be reused for a
/// different handle or identity.
///
/// Format: `domain || name_len(u16 BE) || name || owner(32) || timestamp(u64 BE)`
#[must_use]
pub fn build_pow_challenge(name: &str, owner: &IdentityKey, registered_at: u64) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len() as u16;

    let mut challenge = Vec::with_capacity(POW_DOMAIN.len() + 2 + name_bytes.len() + 32 + 8);
    challenge.extend_from_slice(POW_DOMAIN);
    challenge.extend_from_slice(&name_len.to_be_bytes());
    challenge.extend_from_slice(name_bytes);
    challenge.extend_from_slice(owner.as_bytes());
    challenge.extend_from_slice(&registered_at.to_be_bytes());
    challenge
}

/// Verify that a handle's PoW stamp meets the required difficulty.
///
/// # Errors
///
/// Returns an `EphemeraError::InvalidPow` if the proof is invalid or
/// does not meet the required difficulty.
pub fn verify_handle_pow(
    name: &str,
    owner: &IdentityKey,
    registered_at: u64,
    pow_proof: &PowStamp,
    required_difficulty: PowDifficulty,
) -> Result<(), ephemera_types::EphemeraError> {
    let challenge = build_pow_challenge(name, owner, registered_at);
    pow::verify_pow(pow_proof, &challenge, required_difficulty.bits())
}

/// Build the signature message for a handle record.
///
/// The signed message binds the handle name, owner pubkey, and
/// registration timestamp. This proves that the owner intentionally
/// registered this specific handle at this specific time.
///
/// Format: `domain || name_len(u16 BE) || name || owner(32) || timestamp(u64 BE)`
#[must_use]
pub fn build_signature_message(name: &str, owner: &IdentityKey, registered_at: u64) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len() as u16;

    let mut msg = Vec::with_capacity(SIG_DOMAIN.len() + 2 + name_bytes.len() + 32 + 8);
    msg.extend_from_slice(SIG_DOMAIN);
    msg.extend_from_slice(&name_len.to_be_bytes());
    msg.extend_from_slice(name_bytes);
    msg.extend_from_slice(owner.as_bytes());
    msg.extend_from_slice(&registered_at.to_be_bytes());
    msg
}

/// Verify the Ed25519 signature on a handle record.
///
/// # Errors
///
/// Returns a [`CryptoError`](ephemera_crypto::CryptoError) if the
/// signature is invalid.
pub fn verify_handle_signature(
    name: &str,
    owner: &IdentityKey,
    registered_at: u64,
    signature: &ephemera_types::Signature,
) -> Result<(), ephemera_crypto::CryptoError> {
    let msg = build_signature_message(name, owner, registered_at);
    signing::verify_signature(owner, &msg, signature)
}

#[cfg(test)]
#[path = "handle_validation_tests.rs"]
mod tests;
