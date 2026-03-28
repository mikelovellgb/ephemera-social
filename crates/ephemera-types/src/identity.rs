//! Identity-related types: public keys, signatures, node IDs.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Ed25519 signature length in bytes.
pub const ED25519_SIGNATURE_LEN: usize = 64;

/// An Ed25519 public key used as a pseudonym identity.
///
/// This is the canonical user-layer identity type. It replaces the former
/// `PseudonymId` type which was a source of confusion due to dual naming.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdentityKey([u8; 32]);

impl IdentityKey {
    /// Create an `IdentityKey` from raw bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return the raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Encode the public key as a hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Debug for IdentityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IdentityKey({})", hex::encode(&self.0[..8]))
    }
}

impl fmt::Display for IdentityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// An Ed25519 signature (64 bytes).
///
/// Stored internally as a fixed-size `[u8; 64]` array. Custom serde
/// implementation rejects payloads that are not exactly 64 bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct Signature([u8; ED25519_SIGNATURE_LEN]);

impl Signature {
    /// Create a `Signature` from a fixed-size byte array.
    #[must_use]
    pub fn from_bytes(bytes: [u8; ED25519_SIGNATURE_LEN]) -> Self {
        Self(bytes)
    }

    /// Try to create a `Signature` from a byte slice.
    ///
    /// Returns `None` if the slice is not exactly 64 bytes.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() == ED25519_SIGNATURE_LEN {
            let mut arr = [0u8; ED25519_SIGNATURE_LEN];
            arr.copy_from_slice(bytes);
            Some(Self(arr))
        } else {
            None
        }
    }

    /// Return the raw bytes as a fixed-size array.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; ED25519_SIGNATURE_LEN] {
        self.0
    }

    /// Return the raw bytes as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({}...)", hex::encode(&self.0[..8]))
    }
}

impl Serialize for Signature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a byte sequence (matches the old Vec<u8> wire format).
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SignatureVisitor;

        impl<'de> serde::de::Visitor<'de> for SignatureVisitor {
            type Value = Signature;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a byte sequence of exactly 64 bytes")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Signature::from_slice(v).ok_or_else(|| {
                    E::invalid_length(v.len(), &"exactly 64 bytes for an Ed25519 signature")
                })
            }

            fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_bytes(&v)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut bytes = Vec::with_capacity(ED25519_SIGNATURE_LEN);
                while let Some(byte) = seq.next_element::<u8>()? {
                    if bytes.len() >= ED25519_SIGNATURE_LEN {
                        return Err(serde::de::Error::invalid_length(
                            bytes.len() + 1,
                            &"exactly 64 bytes for an Ed25519 signature",
                        ));
                    }
                    bytes.push(byte);
                }
                Signature::from_slice(&bytes).ok_or_else(|| {
                    serde::de::Error::invalid_length(
                        bytes.len(),
                        &"exactly 64 bytes for an Ed25519 signature",
                    )
                })
            }
        }

        deserializer.deserialize_bytes(SignatureVisitor)
    }
}

/// A 32-byte node identifier on the network (derived from the node's Ed25519 pubkey).
///
/// This is the canonical network-layer identity type. It replaces the former
/// `PeerId` type which was a source of confusion due to dual naming.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId([u8; 32]);

impl NodeId {
    /// Create a `NodeId` from raw bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return the raw bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// XOR distance between two node IDs (used by Kademlia).
    #[must_use]
    pub fn xor_distance(&self, other: &NodeId) -> [u8; 32] {
        let mut dist = [0u8; 32];
        for (i, byte) in dist.iter_mut().enumerate() {
            *byte = self.0[i] ^ other.0[i];
        }
        dist
    }

    /// Return the index of the most-significant bit that differs (0..=255).
    /// Returns `None` if the IDs are identical.
    #[must_use]
    pub fn leading_zeros_distance(&self, other: &NodeId) -> Option<usize> {
        let dist = self.xor_distance(other);
        for (byte_idx, &b) in dist.iter().enumerate() {
            if b != 0 {
                return Some(byte_idx * 8 + b.leading_zeros() as usize);
            }
        }
        None
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({})", hex::encode(&self.0[..8]))
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_distance_is_symmetric() {
        let a = NodeId::from_bytes([1; 32]);
        let b = NodeId::from_bytes([2; 32]);
        assert_eq!(a.xor_distance(&b), b.xor_distance(&a));
    }

    #[test]
    fn xor_distance_to_self_is_zero() {
        let a = NodeId::from_bytes([42; 32]);
        assert_eq!(a.xor_distance(&a), [0u8; 32]);
    }

    #[test]
    fn leading_zeros_distance_identical() {
        let a = NodeId::from_bytes([1; 32]);
        assert_eq!(a.leading_zeros_distance(&a), None);
    }

    #[test]
    fn leading_zeros_distance_differs() {
        let a = NodeId::from_bytes([0; 32]);
        let mut b_bytes = [0u8; 32];
        b_bytes[0] = 0x80; // first bit differs
        let b = NodeId::from_bytes(b_bytes);
        assert_eq!(a.leading_zeros_distance(&b), Some(0));
    }

    #[test]
    fn identity_key_to_hex() {
        let key = IdentityKey::from_bytes([0xCC; 32]);
        assert_eq!(key.to_hex(), hex::encode([0xCC; 32]));
    }

    #[test]
    fn identity_key_is_copy() {
        let key = IdentityKey::from_bytes([0x01; 32]);
        let copy = key; // should compile because Copy
        assert_eq!(key, copy);
    }

    #[test]
    fn signature_from_bytes_round_trip() {
        let sig = Signature::from_bytes([0xAB; 64]);
        assert_eq!(sig.to_bytes(), [0xAB; 64]);
        assert_eq!(sig.as_slice().len(), 64);
    }

    #[test]
    fn signature_from_slice_valid() {
        let bytes = [0x01u8; 64];
        let sig = Signature::from_slice(&bytes).expect("64 bytes should succeed");
        assert_eq!(sig.to_bytes(), bytes);
    }

    #[test]
    fn signature_from_slice_rejects_wrong_length() {
        assert!(Signature::from_slice(&[0u8; 63]).is_none());
        assert!(Signature::from_slice(&[0u8; 65]).is_none());
        assert!(Signature::from_slice(&[]).is_none());
    }

    #[test]
    fn signature_serde_json_round_trip() {
        let sig = Signature::from_bytes([0xDD; 64]);
        let json = serde_json::to_string(&sig).expect("serialize");
        let decoded: Signature = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(sig, decoded);
    }

    #[test]
    fn signature_serde_json_rejects_wrong_length() {
        // A JSON array with 63 bytes should be rejected.
        let short: Vec<u8> = vec![0u8; 63];
        let json = serde_json::to_string(&short).expect("serialize");
        let result = serde_json::from_str::<Signature>(&json);
        assert!(result.is_err());

        // A JSON array with 65 bytes should also be rejected.
        let long: Vec<u8> = vec![0u8; 65];
        let json = serde_json::to_string(&long).expect("serialize");
        let result = serde_json::from_str::<Signature>(&json);
        assert!(result.is_err());
    }
}
