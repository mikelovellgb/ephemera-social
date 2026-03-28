//! Node and pseudonym identity management.
//!
//! Provides high-level identity types that combine a key pair with
//! metadata about its role (network node vs. user pseudonym).

use crate::keys::{derive_pseudonym_key, KeyPair, MasterSecret};
use ed25519_dalek::{Signature, Signer, Verifier};
use ephemera_types::{EphemeraError, IdentityKey, NodeId};

/// A node's network-layer identity.
///
/// Used for DHT participation, gossip membership, and peer-to-peer
/// authentication. This identity is publicly visible on the network
/// and is NOT derived from user key material.
#[derive(Debug)]
pub struct NodeIdentity {
    key_pair: KeyPair,
}

impl NodeIdentity {
    /// Generate a new random node identity.
    #[must_use]
    pub fn generate() -> Self {
        Self {
            key_pair: KeyPair::generate(),
        }
    }

    /// Reconstruct from a stored secret key.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are invalid.
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Result<Self, EphemeraError> {
        Ok(Self {
            key_pair: KeyPair::from_secret_bytes(bytes)?,
        })
    }

    /// The node's [`NodeId`] (public key).
    #[must_use]
    pub fn node_id(&self) -> NodeId {
        self.key_pair.to_node_id()
    }

    /// Deprecated: use [`node_id`](Self::node_id) instead.
    #[deprecated(since = "0.2.0", note = "use `node_id` instead")]
    #[must_use]
    pub fn peer_id(&self) -> NodeId {
        self.node_id()
    }

    /// Sign a message with the node identity key.
    ///
    /// # Errors
    ///
    /// Returns an error if signing fails (should not happen with valid keys).
    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], EphemeraError> {
        let sig: Signature = self.key_pair.signing_key().sign(message);
        Ok(sig.to_bytes())
    }

    /// The underlying key pair (for serialization/backup).
    pub fn key_pair(&self) -> &KeyPair {
        &self.key_pair
    }
}

/// A user's pseudonym identity.
///
/// Derived from the master secret via HKDF. Used for content signing,
/// DM encryption, and social graph operations. Multiple unlinkable
/// pseudonyms can exist per user.
#[derive(Debug)]
pub struct PseudonymIdentity {
    key_pair: KeyPair,
    /// The derivation index used to create this pseudonym.
    index: u32,
}

impl PseudonymIdentity {
    /// Derive a pseudonym identity from a master secret and index.
    ///
    /// # Errors
    ///
    /// Returns an error if key derivation fails.
    pub fn derive(master: &MasterSecret, index: u32) -> Result<Self, EphemeraError> {
        let key_pair = derive_pseudonym_key(master.as_bytes(), index)?;
        Ok(Self { key_pair, index })
    }

    /// The pseudonym's public identifier.
    #[must_use]
    pub fn identity_key(&self) -> IdentityKey {
        self.key_pair.to_identity_key()
    }

    /// Deprecated: use [`identity_key`](Self::identity_key) instead.
    #[deprecated(since = "0.2.0", note = "use `identity_key` instead")]
    #[must_use]
    pub fn pseudonym_id(&self) -> IdentityKey {
        self.identity_key()
    }

    /// The derivation index.
    #[must_use]
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Sign a message with this pseudonym's key.
    ///
    /// # Errors
    ///
    /// Returns an error if signing fails.
    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], EphemeraError> {
        let sig: Signature = self.key_pair.signing_key().sign(message);
        Ok(sig.to_bytes())
    }

    /// The underlying key pair (for serialization/backup).
    pub fn key_pair(&self) -> &KeyPair {
        &self.key_pair
    }
}

/// Verify an Ed25519 signature against a public key.
///
/// # Errors
///
/// Returns [`EphemeraError::SignatureInvalid`] if the signature does not
/// match the message for the given public key.
pub fn verify_signature(
    public_key_bytes: &[u8; 32],
    message: &[u8],
    signature_bytes: &[u8; 64],
) -> Result<(), EphemeraError> {
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(public_key_bytes).map_err(|e| {
        EphemeraError::InvalidKey {
            reason: format!("invalid Ed25519 public key: {e}"),
        }
    })?;

    let signature = Signature::from_bytes(signature_bytes);

    verifying_key
        .verify(message, &signature)
        .map_err(|e| EphemeraError::SignatureInvalid {
            reason: format!("Ed25519 verification failed: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_identity_sign_verify() {
        let node = NodeIdentity::generate();
        let msg = b"hello network";
        let sig = node.sign(msg).unwrap();
        assert!(verify_signature(node.node_id().as_bytes(), msg, &sig).is_ok());
    }

    #[test]
    fn node_identity_bad_signature() {
        let node = NodeIdentity::generate();
        let msg = b"hello network";
        let sig = node.sign(msg).unwrap();
        let result = verify_signature(node.node_id().as_bytes(), b"wrong message", &sig);
        assert!(result.is_err());
    }

    #[test]
    fn pseudonym_identity_derive_and_sign() {
        let master = MasterSecret::generate();
        let pseudo = PseudonymIdentity::derive(&master, 0).unwrap();
        let msg = b"ephemeral post content";
        let sig = pseudo.sign(msg).unwrap();
        assert!(verify_signature(pseudo.identity_key().as_bytes(), msg, &sig).is_ok());
    }

    #[test]
    fn pseudonym_identity_index() {
        let master = MasterSecret::generate();
        let p0 = PseudonymIdentity::derive(&master, 0).unwrap();
        let p1 = PseudonymIdentity::derive(&master, 1).unwrap();
        assert_eq!(p0.index(), 0);
        assert_eq!(p1.index(), 1);
        assert_ne!(p0.identity_key(), p1.identity_key());
    }

    #[test]
    fn node_identity_round_trip() {
        let node = NodeIdentity::generate();
        let secret = node.key_pair().secret_bytes();
        let recovered = NodeIdentity::from_secret_bytes(&secret).unwrap();
        assert_eq!(node.node_id(), recovered.node_id());
    }
}
