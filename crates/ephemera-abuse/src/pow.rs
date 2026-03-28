//! Proof-of-Work generation and verification.
//!
//! Uses a BLAKE3-based PoW scheme (simplified for PoC). The prover must
//! find a nonce such that `BLAKE3(challenge || nonce)` has at least
//! `difficulty` leading zero bits. Verification is O(1).
//!
//! The full system will use Equihash (memory-hard) in production; this
//! module provides the interface and a BLAKE3 fallback for development.

use serde::{Deserialize, Serialize};

use crate::AbuseError;

/// Difficulty presets for different action types.
#[derive(Debug, Clone, Copy)]
pub struct PowDifficulty;

impl PowDifficulty {
    /// Identity creation: ~30 seconds target.
    pub const IDENTITY_CREATION: u32 = 24;
    /// Post or reply creation: ~100ms target.
    pub const POST_CREATION: u32 = 16;
    /// Reaction: ~10ms target.
    pub const REACTION: u32 = 12;
    /// DM to stranger (message request): ~10 seconds.
    pub const MESSAGE_REQUEST: u32 = 22;
    /// Connection request: ~500ms target.
    pub const CONNECTION_REQUEST: u32 = 18;
    /// DM to mutual contact: ~50ms target.
    pub const DM_MUTUAL: u32 = 14;

    /// Adjust difficulty based on network load.
    ///
    /// When the network is under heavy load (many recent actions), the
    /// difficulty is scaled up to deter abuse.
    pub fn adjusted(base: u32, recent_actions: u32) -> u32 {
        let multiplier = match recent_actions {
            0..=10 => 0,
            11..=50 => 1,
            51..=200 => 2,
            _ => 3,
        };
        // Add bits of difficulty, capped at 30 (never exceed ~60s)
        (base + multiplier).min(30)
    }
}

/// A proof-of-work challenge: the data that must be combined with a nonce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowChallenge {
    /// The challenge data (typically a content hash or identity pubkey).
    pub data: Vec<u8>,
    /// Required number of leading zero bits.
    pub difficulty: u32,
}

/// Proof-of-work stamp: proof that computation was performed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowStamp {
    /// The nonce that satisfies the difficulty target.
    pub nonce: [u8; 32],
    /// The resulting hash (for quick verification without recomputing).
    pub hash: [u8; 32],
    /// The difficulty that was targeted.
    pub difficulty: u32,
}

/// Proof-of-Work service for generation and verification.
pub struct ProofOfWork;

impl ProofOfWork {
    /// Generate a proof-of-work for the given challenge.
    ///
    /// Iterates nonces until finding one where
    /// `BLAKE3(challenge.data || nonce)` has at least `challenge.difficulty`
    /// leading zero bits.
    pub fn generate(challenge: &PowChallenge) -> PowStamp {
        let mut nonce_counter: u64 = 0;
        loop {
            let mut nonce = [0u8; 32];
            nonce[..8].copy_from_slice(&nonce_counter.to_le_bytes());

            let hash = Self::compute_hash(&challenge.data, &nonce);

            if Self::count_leading_zeros(&hash) >= challenge.difficulty {
                return PowStamp {
                    nonce,
                    hash,
                    difficulty: challenge.difficulty,
                };
            }
            nonce_counter += 1;
        }
    }

    /// Verify a proof-of-work stamp against the original challenge data.
    pub fn verify(challenge_data: &[u8], stamp: &PowStamp) -> Result<(), AbuseError> {
        // Recompute the hash
        let hash = Self::compute_hash(challenge_data, &stamp.nonce);

        // Verify it matches
        if hash != stamp.hash {
            return Err(AbuseError::PowInvalid {
                reason: "hash mismatch".into(),
            });
        }

        // Verify leading zeros
        let zeros = Self::count_leading_zeros(&hash);
        if zeros < stamp.difficulty {
            return Err(AbuseError::PowInvalid {
                reason: format!(
                    "insufficient difficulty: need {} leading zeros, got {}",
                    stamp.difficulty, zeros
                ),
            });
        }

        Ok(())
    }

    /// Compute `BLAKE3(data || nonce)`.
    fn compute_hash(data: &[u8], nonce: &[u8; 32]) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(data);
        hasher.update(nonce);
        *hasher.finalize().as_bytes()
    }

    /// Count leading zero bits in a 32-byte hash.
    fn count_leading_zeros(hash: &[u8; 32]) -> u32 {
        let mut zeros = 0u32;
        for &byte in hash {
            if byte == 0 {
                zeros += 8;
            } else {
                zeros += byte.leading_zeros();
                break;
            }
        }
        zeros
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_verify_low_difficulty() {
        let challenge = PowChallenge {
            data: b"test-challenge".to_vec(),
            difficulty: 8, // 1 leading zero byte, fast to find
        };

        let stamp = ProofOfWork::generate(&challenge);
        assert!(ProofOfWork::verify(&challenge.data, &stamp).is_ok());
    }

    #[test]
    fn verification_rejects_wrong_data() {
        let challenge = PowChallenge {
            data: b"original".to_vec(),
            difficulty: 8,
        };

        let stamp = ProofOfWork::generate(&challenge);
        let result = ProofOfWork::verify(b"tampered", &stamp);
        assert!(result.is_err());
    }

    #[test]
    fn verification_rejects_insufficient_difficulty() {
        let challenge = PowChallenge {
            data: b"test".to_vec(),
            difficulty: 8,
        };
        let stamp = ProofOfWork::generate(&challenge);

        // Claim a higher difficulty than actually achieved
        let inflated = PowStamp {
            difficulty: 128,
            ..stamp
        };
        let result = ProofOfWork::verify(&challenge.data, &inflated);
        assert!(result.is_err());
    }

    #[test]
    fn leading_zeros_correct() {
        assert_eq!(ProofOfWork::count_leading_zeros(&[0; 32]), 256);
        assert_eq!(ProofOfWork::count_leading_zeros(&[0xFF; 32]), 0);
        let mut h = [0u8; 32];
        h[0] = 0x0F; // 4 leading zeros
        assert_eq!(ProofOfWork::count_leading_zeros(&h), 4);
    }

    #[test]
    fn difficulty_adjustment() {
        let base = PowDifficulty::POST_CREATION;
        assert_eq!(PowDifficulty::adjusted(base, 5), base);
        assert!(PowDifficulty::adjusted(base, 100) > base);
        // Capped at 30
        assert!(PowDifficulty::adjusted(base, 1000) <= 30);
    }
}
