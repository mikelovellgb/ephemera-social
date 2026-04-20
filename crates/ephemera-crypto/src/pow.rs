//! Proof-of-work computation and verification.
//!
//! Uses a **memory-hard** Argon2id-based proof-of-work scheme that cannot
//! be accelerated with GPUs or ASICs. Each PoW attempt requires running
//! Argon2id (32 MiB memory, 2 iterations) to derive a candidate hash,
//! then checking for leading zero bits. This makes each attempt take
//! ~150-250ms on modern hardware regardless of CPU speed.
//!
//! Approximate solve times at each difficulty:
//! - 8 bits: ~256 attempts × 200ms ≈ **50 seconds**
//! - 10 bits: ~1024 attempts × 200ms ≈ **3.5 minutes**
//! - 12 bits: ~4096 attempts × 200ms ≈ **14 minutes**
//!
//! The scheme is bypass-proof: any node can verify a stamp by running the
//! same Argon2id derivation once. Modifying the client to skip Argon2id
//! will produce stamps that fail verification on honest nodes.

use argon2::Argon2;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

/// Argon2id parameters for PoW. Memory-hard to resist GPU acceleration.
/// 32 MiB memory, 2 iterations, 2 lanes.
const POW_ARGON2_M_COST: u32 = 32_768; // 32 MiB in KiB
const POW_ARGON2_T_COST: u32 = 2;
const POW_ARGON2_P_COST: u32 = 2;

/// A proof-of-work stamp attached to content.
///
/// The stamp proves that the sender found a nonce such that
/// `Argon2id(challenge, nonce)` has at least `difficulty` leading zero bits.
/// Each attempt takes ~200ms due to the memory-hard Argon2id function,
/// making the PoW resistant to GPU/ASIC acceleration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowStamp {
    /// The challenge bytes (typically the content hash).
    pub challenge: Vec<u8>,
    /// The nonce that satisfies the difficulty requirement.
    pub nonce: [u8; 16],
    /// The difficulty (number of leading zero bits required).
    pub difficulty: u32,
}

/// Check whether a hash has at least `difficulty` leading zero bits.
fn has_leading_zeros(hash: &[u8], difficulty: u32) -> bool {
    let full_bytes = (difficulty / 8) as usize;
    let remaining_bits = difficulty % 8;

    for &byte in &hash[..full_bytes.min(hash.len())] {
        if byte != 0 {
            return false;
        }
    }

    if remaining_bits > 0 && full_bytes < hash.len() {
        let mask = 0xFFu8 << (8 - remaining_bits);
        if hash[full_bytes] & mask != 0 {
            return false;
        }
    }

    true
}

/// Compute the Argon2id hash for a PoW attempt.
///
/// Uses the challenge as the "password" and the nonce as the "salt".
/// Returns a 32-byte hash that takes ~200ms to compute.
fn pow_hash(challenge: &[u8], nonce: &[u8; 16]) -> [u8; 32] {
    let argon2 = Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(POW_ARGON2_M_COST, POW_ARGON2_T_COST, POW_ARGON2_P_COST, Some(32))
            .expect("valid Argon2 params"),
    );

    let mut output = [0u8; 32];
    argon2
        .hash_password_into(challenge, nonce, &mut output)
        .expect("Argon2id hash should not fail");
    output
}

/// Generate a proof-of-work stamp for the given challenge and difficulty.
///
/// This function will loop until it finds a valid nonce. Each iteration
/// takes ~200ms due to Argon2id. The expected number of iterations is
/// `2^difficulty`, so:
/// - difficulty 8: ~50 seconds
/// - difficulty 10: ~3.5 minutes
/// - difficulty 12: ~14 minutes
///
/// # Arguments
///
/// * `challenge` - The challenge bytes (binds handle name + owner + timestamp).
/// * `difficulty` - Number of leading zero bits required.
///
/// # Panics
///
/// Panics if `difficulty` exceeds 256.
#[must_use]
pub fn generate_pow(challenge: &[u8], difficulty: u32) -> PowStamp {
    assert!(difficulty <= 256, "difficulty cannot exceed 256 bits");

    let mut nonce = [0u8; 16];
    let mut counter: u64 = 0;
    loop {
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
/// Runs Argon2id once (~200ms) to verify the stamp is valid. This is
/// intentionally expensive to prevent flooding with invalid stamps.
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
        // Use difficulty 1 for fast tests (~2 Argon2id calls, ~400ms)
        let challenge = b"test challenge data";
        let stamp = generate_pow(challenge, 1);
        assert!(verify_pow(&stamp, challenge, 1).is_ok());
    }

    #[test]
    fn verify_rejects_wrong_challenge() {
        let stamp = generate_pow(b"real challenge", 1);
        assert!(verify_pow(&stamp, b"fake challenge", 1).is_err());
    }

    #[test]
    fn verify_rejects_low_difficulty() {
        let stamp = generate_pow(b"challenge", 1);
        assert!(verify_pow(&stamp, b"challenge", 4).is_err());
    }

    #[test]
    fn zero_difficulty() {
        let stamp = generate_pow(b"easy", 0);
        assert!(verify_pow(&stamp, b"easy", 0).is_ok());
    }
}
