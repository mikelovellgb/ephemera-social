//! X3DH (Extended Triple Diffie-Hellman) key exchange.
//!
//! Implements the Signal-style X3DH protocol adapted for Ephemera.
//! The initiator computes a shared secret from 3 or 4 DH operations
//! depending on whether a one-time prekey is available. The responder
//! computes the same shared secret from their private keys and the
//! initiator's public keys sent in the initial message.
//!
//! Note: In Ephemera, each identity has both an Ed25519 key (for
//! signing) and a separate X25519 key (for DH). The identity_x25519
//! key is the long-term DH key published in the prekey bundle.
//!
//! Reference: <https://signal.org/docs/specifications/x3dh/>

use ephemera_crypto::{
    keys::hkdf_derive, x25519_diffie_hellman, X25519KeyPair, X25519PublicKey, X25519SecretKey,
};
use ephemera_types::IdentityKey;
use zeroize::Zeroize;

use crate::prekey::PrekeyBundle;
use crate::MessageError;

/// HKDF info string for X3DH shared secret derivation.
const X3DH_INFO: &[u8] = b"ephemera-x3dh-v1";

/// 32 bytes of zeros used as salt for HKDF (per Signal spec).
const X3DH_SALT: [u8; 32] = [0u8; 32];

/// The data an initiator sends alongside the first message so the
/// responder can compute the same shared secret.
#[derive(Debug, Clone)]
pub struct X3dhInitialMessage {
    /// The initiator's Ed25519 identity key (for identification).
    pub identity_key: IdentityKey,
    /// The initiator's long-term X25519 public key (for DH1).
    pub identity_x25519: X25519PublicKey,
    /// The initiator's ephemeral X25519 public key (generated fresh).
    pub ephemeral_key: X25519PublicKey,
    /// Which signed prekey was used (for the responder to look up).
    pub signed_prekey_used: X25519PublicKey,
    /// Which one-time prekey was consumed (if any).
    pub one_time_prekey_used: Option<X25519PublicKey>,
}

/// Result of the initiator's X3DH computation.
pub struct X3dhInitiatorResult {
    /// The 32-byte shared secret (used to initialize the ratchet).
    pub shared_secret: [u8; 32],
    /// The initial message header to send to the responder.
    pub initial_message: X3dhInitialMessage,
}

impl Drop for X3dhInitiatorResult {
    fn drop(&mut self) {
        self.shared_secret.zeroize();
    }
}

/// Initiator side of X3DH: compute a shared secret from the
/// recipient's prekey bundle.
///
/// Performs 3 (or 4, if one-time prekey is available) DH operations
/// and derives the shared secret via HKDF-SHA256.
///
/// # Arguments
/// * `our_identity_x25519` - Our long-term X25519 secret key (for DH)
/// * `our_identity_x25519_pub` - Our long-term X25519 public key
/// * `our_identity_ed25519` - Our Ed25519 identity key (for identification)
/// * `their_bundle` - The recipient's published prekey bundle
pub fn x3dh_initiate(
    our_identity_x25519: &X25519SecretKey,
    our_identity_x25519_pub: &X25519PublicKey,
    our_identity_ed25519: &IdentityKey,
    their_bundle: &PrekeyBundle,
) -> Result<X3dhInitiatorResult, MessageError> {
    // Generate a fresh ephemeral keypair for this session.
    let ephemeral = X25519KeyPair::generate();

    // DH1: our identity X25519 * their signed prekey
    let dh1 = x25519_diffie_hellman(our_identity_x25519, &their_bundle.signed_prekey);

    // DH2: our ephemeral * their identity X25519
    let dh2 = x25519_diffie_hellman(&ephemeral.secret, &their_bundle.identity_x25519);

    // DH3: our ephemeral * their signed prekey
    let dh3 = x25519_diffie_hellman(&ephemeral.secret, &their_bundle.signed_prekey);

    // Concatenate DH results.
    let mut dh_concat = Vec::with_capacity(128);
    dh_concat.extend_from_slice(&*dh1);
    dh_concat.extend_from_slice(&*dh2);
    dh_concat.extend_from_slice(&*dh3);

    // DH4: our ephemeral * their one-time prekey (if available).
    let one_time_prekey_used = if let Some(ref otpk) = their_bundle.one_time_prekey {
        let dh4 = x25519_diffie_hellman(&ephemeral.secret, otpk);
        dh_concat.extend_from_slice(&*dh4);
        Some(otpk.clone())
    } else {
        None
    };

    // Derive shared secret via HKDF-SHA256.
    let shared_secret = hkdf_derive(&dh_concat, Some(&X3DH_SALT), X3DH_INFO).map_err(|e| {
        MessageError::X3dhFailed {
            reason: format!("HKDF derivation failed: {e}"),
        }
    })?;

    // Zeroize the intermediate DH concatenation.
    dh_concat.zeroize();

    let initial_message = X3dhInitialMessage {
        identity_key: *our_identity_ed25519,
        identity_x25519: our_identity_x25519_pub.clone(),
        ephemeral_key: ephemeral.public.clone(),
        signed_prekey_used: their_bundle.signed_prekey.clone(),
        one_time_prekey_used,
    };

    Ok(X3dhInitiatorResult {
        shared_secret,
        initial_message,
    })
}

/// Responder side of X3DH: compute the same shared secret from
/// the initiator's initial message and our private keys.
///
/// # Arguments
/// * `our_identity_x25519` - Our long-term X25519 secret key
/// * `our_signed_prekey_secret` - The secret key for our signed prekey
/// * `our_one_time_prekey_secret` - The secret for the consumed OTP (if used)
/// * `initial_msg` - The X3DH header from the initiator
pub fn x3dh_respond(
    our_identity_x25519: &X25519SecretKey,
    our_signed_prekey_secret: &X25519SecretKey,
    our_one_time_prekey_secret: Option<&X25519SecretKey>,
    initial_msg: &X3dhInitialMessage,
) -> Result<[u8; 32], MessageError> {
    // DH1: their identity X25519 * our signed prekey (mirrors initiator DH1)
    let dh1 = x25519_diffie_hellman(our_signed_prekey_secret, &initial_msg.identity_x25519);

    // DH2: their ephemeral * our identity X25519 (mirrors initiator DH2)
    let dh2 = x25519_diffie_hellman(our_identity_x25519, &initial_msg.ephemeral_key);

    // DH3: their ephemeral * our signed prekey (mirrors initiator DH3)
    let dh3 = x25519_diffie_hellman(our_signed_prekey_secret, &initial_msg.ephemeral_key);

    let mut dh_concat = Vec::with_capacity(128);
    dh_concat.extend_from_slice(&*dh1);
    dh_concat.extend_from_slice(&*dh2);
    dh_concat.extend_from_slice(&*dh3);

    // DH4: their ephemeral * our one-time prekey (if used)
    if initial_msg.one_time_prekey_used.is_some() {
        let otp_secret = our_one_time_prekey_secret.ok_or_else(|| MessageError::X3dhFailed {
            reason: "initiator used one-time prekey but no matching secret".into(),
        })?;
        let dh4 = x25519_diffie_hellman(otp_secret, &initial_msg.ephemeral_key);
        dh_concat.extend_from_slice(&*dh4);
    }

    let shared_secret = hkdf_derive(&dh_concat, Some(&X3DH_SALT), X3DH_INFO).map_err(|e| {
        MessageError::X3dhFailed {
            reason: format!("HKDF derivation failed: {e}"),
        }
    })?;

    dh_concat.zeroize();

    Ok(shared_secret)
}

#[cfg(test)]
#[path = "x3dh_tests.rs"]
mod tests;
