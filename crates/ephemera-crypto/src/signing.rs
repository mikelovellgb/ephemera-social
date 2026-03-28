//! Ed25519 signing and verification.

use crate::CryptoError;
use ed25519_dalek::{Signer, Verifier};
use ephemera_types::{IdentityKey, Signature};
use rand::rngs::OsRng;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// An Ed25519 signing keypair.
///
/// The 32-byte secret seed is stored directly and zeroized on drop.
/// The `SigningKey` is reconstructed on demand.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SigningKeyPair {
    seed: [u8; 32],
}

impl SigningKeyPair {
    /// Generate a new random keypair.
    #[must_use]
    pub fn generate() -> Self {
        let key = ed25519_dalek::SigningKey::generate(&mut OsRng);
        Self {
            seed: key.to_bytes(),
        }
    }

    /// Restore from raw 32-byte secret key bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self { seed: *bytes }
    }

    /// Return the public key as an `IdentityKey`.
    pub fn public_key(&self) -> IdentityKey {
        let key = ed25519_dalek::SigningKey::from_bytes(&self.seed);
        IdentityKey::from_bytes(key.verifying_key().to_bytes())
    }

    /// Return the secret key bytes wrapped in [`Zeroizing`] so the
    /// caller's copy is automatically wiped on drop.
    #[must_use]
    pub fn secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.seed)
    }

    /// Sign a message.
    pub fn sign(&self, message: &[u8]) -> Signature {
        let key = ed25519_dalek::SigningKey::from_bytes(&self.seed);
        let sig = key.sign(message);
        Signature::from_bytes(sig.to_bytes())
    }
}

/// Verify an Ed25519 signature.
pub fn verify_signature(
    public_key: &IdentityKey,
    message: &[u8],
    signature: &Signature,
) -> Result<(), CryptoError> {
    let verifying_key =
        ed25519_dalek::VerifyingKey::from_bytes(public_key.as_bytes()).map_err(|_| {
            CryptoError::InvalidKey {
                reason: "invalid Ed25519 public key".into(),
            }
        })?;
    let sig = ed25519_dalek::Signature::from_bytes(&signature.to_bytes());
    verifying_key
        .verify(message, &sig)
        .map_err(|_| CryptoError::SignatureInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let kp = SigningKeyPair::generate();
        let msg = b"hello ephemera";
        let sig = kp.sign(msg);
        assert!(verify_signature(&kp.public_key(), msg, &sig).is_ok());
    }

    #[test]
    fn bad_signature_fails() {
        let kp = SigningKeyPair::generate();
        let sig = kp.sign(b"correct message");
        assert!(verify_signature(&kp.public_key(), b"wrong message", &sig).is_err());
    }
}
