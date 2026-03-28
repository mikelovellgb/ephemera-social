//! Key pair generation, derivation, and secure memory handling.
//!
//! All key material implements [`Zeroize`] so that secrets are wiped
//! from memory when dropped.

use ed25519_dalek::{SigningKey, VerifyingKey};
use ephemera_types::{EphemeraError, IdentityKey, NodeId};
use hkdf::Hkdf;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// An Ed25519 signing key pair with secure memory handling.
///
/// The 32-byte secret seed is stored directly (not as a `SigningKey`)
/// so that it is properly zeroized on drop. The `SigningKey` is
/// reconstructed on demand when signing operations are needed.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct KeyPair {
    seed: [u8; 32],
}

impl KeyPair {
    /// Generate a fresh random key pair using the OS CSPRNG.
    #[must_use]
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self {
            seed: signing_key.to_bytes(),
        }
    }

    /// Reconstruct a key pair from the 32-byte secret key seed.
    ///
    /// # Errors
    ///
    /// Returns [`EphemeraError::InvalidKey`] if the bytes are not a valid
    /// Ed25519 secret key.
    pub fn from_secret_bytes(bytes: &[u8; 32]) -> Result<Self, EphemeraError> {
        // Validate: constructing a SigningKey from these bytes must succeed.
        // Ed25519-dalek accepts any 32-byte seed, so this is infallible,
        // but we keep the Result signature for forward compatibility.
        let _key = SigningKey::from_bytes(bytes);
        Ok(Self { seed: *bytes })
    }

    /// The 32-byte secret key seed wrapped in [`Zeroizing`] so the
    /// caller's copy is automatically wiped on drop.
    #[must_use]
    pub fn secret_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.seed)
    }

    /// The Ed25519 public (verifying) key.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key_inner().verifying_key()
    }

    /// The 32-byte public key bytes.
    #[must_use]
    pub fn public_bytes(&self) -> [u8; 32] {
        self.verifying_key().to_bytes()
    }

    /// Convert the public key to a [`NodeId`] (for node identity).
    #[must_use]
    pub fn to_node_id(&self) -> NodeId {
        NodeId::from_bytes(self.public_bytes())
    }

    /// Convert the public key to an [`IdentityKey`] (for user identity).
    #[must_use]
    pub fn to_identity_key(&self) -> IdentityKey {
        IdentityKey::from_bytes(self.public_bytes())
    }

    /// Deprecated: use [`to_node_id`](Self::to_node_id) instead.
    #[deprecated(since = "0.2.0", note = "use `to_node_id` instead")]
    #[must_use]
    pub fn to_peer_id(&self) -> NodeId {
        self.to_node_id()
    }

    /// Deprecated: use [`to_identity_key`](Self::to_identity_key) instead.
    #[deprecated(since = "0.2.0", note = "use `to_identity_key` instead")]
    #[must_use]
    pub fn to_pseudonym_id(&self) -> IdentityKey {
        self.to_identity_key()
    }

    /// Reconstruct the `SigningKey` from the stored seed.
    ///
    /// This is intentionally not cached so that only the raw seed
    /// (which implements `Zeroize`) is held long-term.
    pub(crate) fn signing_key_inner(&self) -> SigningKey {
        SigningKey::from_bytes(&self.seed)
    }

    /// Access a transient `SigningKey` for signing operations.
    pub(crate) fn signing_key(&self) -> SigningKey {
        self.signing_key_inner()
    }
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeyPair(pub={})", hex::encode(self.public_bytes()))
    }
}

/// HKDF-SHA256 key derivation info strings.
pub mod derivation {
    /// Info string for deriving pseudonym keys from the master secret.
    pub const PSEUDONYM_INFO_PREFIX: &[u8] = b"ephemera-pseudonym";

    /// Info string for deriving device keys from the master secret.
    pub const DEVICE_INFO_PREFIX: &[u8] = b"ephemera-device";

    /// Info string for deriving epoch encryption keys.
    pub const EPOCH_INFO_PREFIX: &[u8] = b"ephemera-epoch";

    /// Info string for deriving session keys.
    pub const SESSION_INFO_PREFIX: &[u8] = b"ephemera-session";
}

/// Derive a 32-byte key using HKDF-SHA256.
///
/// # Arguments
///
/// * `ikm` - Input key material (the master secret).
/// * `salt` - Optional salt (use `None` for default).
/// * `info` - Context and application-specific information.
///
/// # Errors
///
/// Returns [`EphemeraError::KeyDerivationError`] if HKDF expansion fails
/// (should not happen with valid parameters).
pub fn hkdf_derive(
    ikm: &[u8],
    salt: Option<&[u8]>,
    info: &[u8],
) -> Result<[u8; 32], EphemeraError> {
    let hk = Hkdf::<Sha256>::new(salt, ikm);
    let mut okm = [0u8; 32];
    hk.expand(info, &mut okm)
        .map_err(|e| EphemeraError::KeyDerivationError {
            reason: format!("HKDF expansion failed: {e}"),
        })?;
    Ok(okm)
}

/// Derive a pseudonym signing key from a master secret and an index.
///
/// The derivation path is:
/// `HKDF-SHA256(master, "ephemera-pseudonym" || index_le_bytes)`.
///
/// # Errors
///
/// Returns an error if key derivation fails.
pub fn derive_pseudonym_key(master_secret: &[u8], index: u32) -> Result<KeyPair, EphemeraError> {
    let mut info = Vec::with_capacity(derivation::PSEUDONYM_INFO_PREFIX.len() + 4);
    info.extend_from_slice(derivation::PSEUDONYM_INFO_PREFIX);
    info.extend_from_slice(&index.to_le_bytes());

    let seed = Zeroizing::new(hkdf_derive(master_secret, None, &info)?);
    KeyPair::from_secret_bytes(&seed)
}

/// Derive an epoch encryption key from a master secret and epoch number.
///
/// # Errors
///
/// Returns an error if key derivation fails.
pub fn derive_epoch_key(master_secret: &[u8], epoch: u64) -> Result<[u8; 32], EphemeraError> {
    let mut info = Vec::with_capacity(derivation::EPOCH_INFO_PREFIX.len() + 8);
    info.extend_from_slice(derivation::EPOCH_INFO_PREFIX);
    info.extend_from_slice(&epoch.to_le_bytes());

    hkdf_derive(master_secret, None, &info)
}

/// A master secret with zeroize-on-drop semantics.
///
/// This wraps the raw 32-byte master key material that serves as the
/// root of the key hierarchy.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterSecret {
    bytes: [u8; 32],
}

impl MasterSecret {
    /// Create from raw bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// Generate a fresh random master secret.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut OsRng, &mut bytes);
        Self { bytes }
    }

    /// Access the raw bytes (sensitive!).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }
}

impl std::fmt::Debug for MasterSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MasterSecret([REDACTED])")
    }
}

/// Convert an Ed25519 signing key seed to an X25519 secret key.
///
/// Uses the birational map from Ed25519 to Curve25519 (Montgomery form).
/// The resulting X25519 keypair can be used for Diffie-Hellman key exchange
/// in the messaging layer.
pub fn ed25519_seed_to_x25519(
    ed25519_seed: &[u8; 32],
) -> crate::dh::X25519KeyPair {
    let signing_key = SigningKey::from_bytes(ed25519_seed);
    let scalar_bytes = signing_key.to_scalar_bytes();
    crate::dh::X25519KeyPair::from_secret_bytes(&scalar_bytes)
}

/// Convert an Ed25519 public key (verifying key) to an X25519 public key.
///
/// Uses the birational map from the Edwards curve to the Montgomery form.
/// This lets any node compute the X25519 public key for DH key exchange
/// given only the peer's Ed25519 identity key, without needing an extra
/// key publication step.
///
/// # Errors
///
/// Returns an error if the Ed25519 public key bytes are invalid.
pub fn ed25519_pub_to_x25519(
    ed25519_pub: &[u8; 32],
) -> Result<crate::dh::X25519PublicKey, EphemeraError> {
    let verifying_key =
        VerifyingKey::from_bytes(ed25519_pub).map_err(|_| EphemeraError::InvalidKey {
            reason: "invalid Ed25519 public key for X25519 conversion".into(),
        })?;
    let montgomery = verifying_key.to_montgomery();
    Ok(crate::dh::X25519PublicKey::from_bytes(montgomery.to_bytes()))
}

/// Serializable representation of an Ed25519 public key, for wire and storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicKeyBytes(#[serde(with = "hex_bytes")] pub [u8; 32]);

/// Helper module for serializing `[u8; 32]` as hex strings in JSON.
mod hex_bytes {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom(
                "expected 32 bytes for public key hex string",
            ));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
