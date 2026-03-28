//! Proof-of-work computation and verification.
//!
//! Implements a simplified Hashcash-style proof-of-work scheme using
//! BLAKE3 as the hash function. The PoW stamp proves that the sender
//! expended computational effort, which helps deter spam and abuse.

use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

/// A proof-of-work stamp attached to content.
///
/// The stamp proves that the sender found a nonce such that
/// `BLAKE3(challenge || nonce)` has at least `difficulty` leading zero bits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowStamp {
    /// The challenge bytes (typically the content hash).
    pub challenge: Vec<u8>,
    /// The nonce that satisfies the difficulty requirement.
    pub nonce: [u8; 16],
    /// The difficulty (number of leading zero bits required).
    pub difficulty: u32,
}

/// Check whether a BLAKE3 hash has at least `difficulty` leading zero bits.
fn has_leading_zeros(hash: &[u8; 32], difficulty: u32) -> bool {
    let full_bytes = (difficulty / 8) as usize;
    let remaining_bits = difficulty % 8;

    for &byte in &hash[..full_bytes.min(32)] {
        if byte != 0 {
            return false;
        }
    }

    if remaining_bits > 0 && full_bytes < 32 {
        let mask = 0xFFu8 << (8 - remaining_bits);
        if hash[full_bytes] & mask != 0 {
            return false;
        }
    }

    true
}

/// Compute the BLAKE3 hash for a PoW attempt.
fn pow_hash(challenge: &[u8], nonce: &[u8; 16]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(challenge);
    hasher.update(nonce);
    let hash = hasher.finalize();
    *hash.as_bytes()
}

/// Generate a proof-of-work stamp for the given challenge and difficulty.
///
/// This function will loop until it finds a valid nonce. The expected
/// number of iterations is `2^difficulty`.
///
/// # Arguments
///
/// * `challenge` - The challenge bytes (typically the content hash).
/// * `difficulty` - Number of leading zero bits required (keep this small
///   for interactive use, e.g. 16-20).
///
/// # Panics
///
/// Panics if `difficulty` exceeds 256 (the BLAKE3 output size in bits).
#[must_use]
pub fn generate_pow(challenge: &[u8], difficulty: u32) -> PowStamp {
    assert!(difficulty <= 256, "difficulty cannot exceed 256 bits");

    let mut nonce = [0u8; 16];
    let mut counter: u64 = 0;
    loop {
        // Use a deterministic counter for the nonce to avoid wasting entropy.
        nonce[..8].copy_from_slice(&counter.to_le_bytes());
        let hash = pow_hash(challenge, &nonce);
        if has_leading_zeros(&hash, difficulty) {
            return PowStamp {
                challenge: challenge.to_vec(),
                nonce,
                difficulty,
            };
        }
        counter += 1;
    }
}

/// Verify a proof-of-work stamp.
///
/// # Errors
///
/// Returns `EphemeraError::InvalidPow` if the stamp is invalid.
pub fn verify_pow(
    stamp: &PowStamp,
    expected_challenge: &[u8],
    min_difficulty: u32,
) -> Result<(), ephemera_types::EphemeraError> {
    if stamp.challenge.ct_eq(expected_challenge).unwrap_u8() != 1 {
        return Err(ephemera_types::EphemeraError::InvalidPow {
            reason: "challenge mismatch".into(),
        });
    }

    if stamp.difficulty < min_difficulty {
        return Err(ephemera_types::EphemeraError::InvalidPow {
            reason: format!(
                "difficulty {} is below minimum {}",
                stamp.difficulty, min_difficulty
            ),
        });
    }

    let hash = pow_hash(&stamp.challenge, &stamp.nonce);
    if !has_leading_zeros(&hash, stamp.difficulty) {
        return Err(ephemera_types::EphemeraError::InvalidPow {
            reason: "hash does not meet difficulty requirement".into(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_zeros_check() {
        let hash = [0u8; 32];
        assert!(has_leading_zeros(&hash, 0));
        assert!(has_leading_zeros(&hash, 8));
        assert!(has_leading_zeros(&hash, 256));

        let mut hash2 = [0u8; 32];
        hash2[0] = 0x01;
        assert!(has_leading_zeros(&hash2, 7));
        assert!(!has_leading_zeros(&hash2, 8));
    }

    #[test]
    fn generate_and_verify_low_difficulty() {
        let challenge = b"test challenge data";
        let stamp = generate_pow(challenge, 8);
        assert!(verify_pow(&stamp, challenge, 8).is_ok());
    }

    #[test]
    fn verify_rejects_wrong_challenge() {
        let stamp = generate_pow(b"real challenge", 8);
        assert!(verify_pow(&stamp, b"fake challenge", 8).is_err());
    }

    #[test]
    fn verify_rejects_low_difficulty() {
        let stamp = generate_pow(b"challenge", 8);
        assert!(verify_pow(&stamp, b"challenge", 16).is_err());
    }

    #[test]
    fn zero_difficulty() {
        let stamp = generate_pow(b"easy", 0);
        assert!(verify_pow(&stamp, b"easy", 0).is_ok());
    }
}
