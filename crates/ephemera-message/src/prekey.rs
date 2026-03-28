//! Prekey bundles for asynchronous X3DH key exchange.
//!
//! Each pseudonym publishes a prekey bundle so that other users can
//! initiate encrypted conversations even when the recipient is offline.
//! The bundle contains a long-term identity key (Ed25519 for signing),
//! a long-term X25519 key (for DH), a signed medium-term prekey
//! (rotated weekly), and optional one-time prekeys for additional
//! forward secrecy.

use ephemera_crypto::{
    signing::{verify_signature, SigningKeyPair},
    X25519KeyPair, X25519PublicKey,
};
use ephemera_types::{IdentityKey, Signature};

use crate::MessageError;

/// A published prekey bundle for asynchronous X3DH key exchange.
///
/// The identity_key is the long-term Ed25519 public key (signing).
/// The identity_x25519 is the long-term X25519 public key (DH).
/// The signed_prekey is a medium-term X25519 key signed by the identity
/// key. The one_time_prekey is an optional single-use X25519 key
/// consumed on first message.
#[derive(Debug, Clone)]
pub struct PrekeyBundle {
    /// Long-term Ed25519 identity key (for signing and identification).
    pub identity_key: IdentityKey,
    /// Long-term X25519 public key (for DH operations in X3DH).
    pub identity_x25519: X25519PublicKey,
    /// Signed medium-term X25519 prekey (rotated every 7 days).
    pub signed_prekey: X25519PublicKey,
    /// Ed25519 signature over the signed_prekey bytes.
    pub signed_prekey_signature: Signature,
    /// Optional one-time X25519 prekey (consumed on use).
    pub one_time_prekey: Option<X25519PublicKey>,
}

/// Generate a prekey bundle for the given identity.
///
/// Creates a fresh signed prekey (X25519) and signs it with the Ed25519
/// identity key. The caller must provide the long-term X25519 public key
/// separately. Optionally includes a one-time prekey for extra forward
/// secrecy.
pub fn generate_prekey_bundle(
    signing_key: &SigningKeyPair,
    identity_x25519_pub: &X25519PublicKey,
    include_one_time: bool,
) -> PrekeyBundle {
    let identity_key = signing_key.public_key();
    let signed_prekey_pair = X25519KeyPair::generate();
    let signed_prekey = signed_prekey_pair.public.clone();

    // Sign the raw bytes of the signed prekey with the Ed25519 identity key.
    let signature = signing_key.sign(signed_prekey.as_bytes());

    let one_time_prekey = if include_one_time {
        Some(X25519KeyPair::generate().public)
    } else {
        None
    };

    PrekeyBundle {
        identity_key,
        identity_x25519: identity_x25519_pub.clone(),
        signed_prekey,
        signed_prekey_signature: signature,
        one_time_prekey,
    }
}

/// Validate a prekey bundle by verifying the signed_prekey_signature.
///
/// Returns `Ok(())` if the signature is valid, or an error describing
/// why validation failed.
pub fn validate_prekey_bundle(bundle: &PrekeyBundle) -> Result<(), MessageError> {
    verify_signature(
        &bundle.identity_key,
        bundle.signed_prekey.as_bytes(),
        &bundle.signed_prekey_signature,
    )
    .map_err(|_| MessageError::InvalidPrekeyBundle {
        reason: "signed prekey signature verification failed".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_validate_bundle() {
        let signing = SigningKeyPair::generate();
        let x25519 = X25519KeyPair::generate();
        let bundle = generate_prekey_bundle(&signing, &x25519.public, true);

        assert!(validate_prekey_bundle(&bundle).is_ok());
        assert!(bundle.one_time_prekey.is_some());
    }

    #[test]
    fn test_bundle_without_one_time_key() {
        let signing = SigningKeyPair::generate();
        let x25519 = X25519KeyPair::generate();
        let bundle = generate_prekey_bundle(&signing, &x25519.public, false);

        assert!(validate_prekey_bundle(&bundle).is_ok());
        assert!(bundle.one_time_prekey.is_none());
    }

    #[test]
    fn test_prekey_bundle_validation_tampered() {
        let signing = SigningKeyPair::generate();
        let x25519 = X25519KeyPair::generate();
        let mut bundle = generate_prekey_bundle(&signing, &x25519.public, true);

        // Tamper with the signed prekey (replace with a different key).
        bundle.signed_prekey = X25519KeyPair::generate().public;

        assert!(validate_prekey_bundle(&bundle).is_err());
    }

    #[test]
    fn test_prekey_bundle_wrong_identity() {
        let signing1 = SigningKeyPair::generate();
        let signing2 = SigningKeyPair::generate();
        let x25519 = X25519KeyPair::generate();
        let mut bundle = generate_prekey_bundle(&signing1, &x25519.public, false);

        // Replace identity key with a different one (signature won't match).
        bundle.identity_key = signing2.public_key();

        assert!(validate_prekey_bundle(&bundle).is_err());
    }
}
