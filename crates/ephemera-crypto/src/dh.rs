//! X25519 Diffie-Hellman key exchange.
//!
//! Used by the messaging subsystem for ephemeral key exchange and
//! shared secret derivation. Phase 2 will integrate this with the
//! full Double Ratchet protocol.

use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, Zeroizing};

/// An X25519 public key (32 bytes).
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct X25519PublicKey([u8; 32]);

impl X25519PublicKey {
    /// Create from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for X25519PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "X25519Pub({})", hex::encode(&self.0[..8]))
    }
}

/// An X25519 secret key (32 bytes). Zeroized on drop.
pub struct X25519SecretKey {
    bytes: [u8; 32],
}

impl X25519SecretKey {
    /// Create from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// Return the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl Drop for X25519SecretKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

/// An X25519 keypair.
pub struct X25519KeyPair {
    /// The secret key.
    pub secret: X25519SecretKey,
    /// The public key.
    pub public: X25519PublicKey,
}

impl X25519KeyPair {
    /// Generate a new random X25519 keypair.
    #[must_use]
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let public = PublicKey::from(&secret);
        let mut secret_bytes = secret.to_bytes();
        let result = Self {
            secret: X25519SecretKey {
                bytes: secret_bytes,
            },
            public: X25519PublicKey::from_bytes(public.to_bytes()),
        };
        secret_bytes.zeroize();
        result
    }

    /// Restore from raw secret key bytes (by reference to avoid extra copies).
    #[must_use]
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Self {
        let secret = StaticSecret::from(*bytes);
        let public = PublicKey::from(&secret);
        let mut sb = secret.to_bytes();
        let result = Self {
            secret: X25519SecretKey { bytes: sb },
            public: X25519PublicKey::from_bytes(public.to_bytes()),
        };
        sb.zeroize();
        result
    }
}

/// Perform X25519 Diffie-Hellman, returning the 32-byte shared secret
/// wrapped in [`Zeroizing`] so it is wiped on drop.
pub fn x25519_diffie_hellman(
    our_secret: &X25519SecretKey,
    their_public: &X25519PublicKey,
) -> Zeroizing<[u8; 32]> {
    let secret = StaticSecret::from(*our_secret.as_bytes());
    let public = PublicKey::from(*their_public.as_bytes());
    Zeroizing::new(*secret.diffie_hellman(&public).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dh_shared_secret_symmetric() {
        let alice = X25519KeyPair::generate();
        let bob = X25519KeyPair::generate();
        let shared_a = x25519_diffie_hellman(&alice.secret, &bob.public);
        let shared_b = x25519_diffie_hellman(&bob.secret, &alice.public);
        assert_eq!(shared_a, shared_b);
    }

    #[test]
    fn keypair_from_secret_bytes() {
        let kp1 = X25519KeyPair::generate();
        let kp2 = X25519KeyPair::from_secret_bytes(kp1.secret.as_bytes());
        assert_eq!(kp1.public, kp2.public);
    }
}
