//! Simplified Double Ratchet providing forward secrecy and
//! post-compromise security.
//!
//! Each message is encrypted with a unique key derived from a chain key
//! that advances on every send/receive (symmetric ratchet). A DH ratchet
//! step occurs on each reply, rotating the root key material so that
//! compromise of current keys does not reveal past messages.
//!
//! Reference: <https://signal.org/docs/specifications/doubleratchet/>

use ephemera_crypto::{
    encryption::{open, seal},
    keys::hkdf_derive,
    x25519_diffie_hellman, X25519KeyPair, X25519PublicKey,
};
use zeroize::Zeroize;

use crate::MessageError;

/// HKDF info for advancing the chain key (symmetric ratchet).
const CHAIN_INFO: &[u8] = b"ephemera-chain-v1";

/// HKDF info for deriving a per-message encryption key.
const MSG_KEY_INFO: &[u8] = b"ephemera-msg-v1";

/// HKDF info for the DH ratchet step (root key update).
const RATCHET_INFO: &[u8] = b"ephemera-ratchet-v1";

/// Header sent with each ratchet-encrypted message.
///
/// Contains the sender's current ratchet public key and the message
/// counter so the receiver can derive the correct decryption key.
#[derive(Debug, Clone)]
pub struct MessageHeader {
    /// Sender's current DH ratchet public key.
    pub ratchet_key: X25519PublicKey,
    /// Message number in the current sending chain.
    pub message_number: u32,
    /// Number of messages in the previous sending chain (for skip).
    pub previous_chain_length: u32,
}

/// The Double Ratchet state for one side of a conversation.
///
/// Holds root key, send/receive chain keys, DH ratchet keys, and
/// message counters. All secret material is zeroized on drop.
pub struct RatchetState {
    root_key: [u8; 32],
    pub(crate) send_chain_key: [u8; 32],
    pub(crate) recv_chain_key: [u8; 32],
    send_ratchet_key: X25519KeyPair,
    recv_ratchet_pubkey: Option<X25519PublicKey>,
    send_count: u32,
    recv_count: u32,
    previous_send_count: u32,
}

impl Drop for RatchetState {
    fn drop(&mut self) {
        self.root_key.zeroize();
        self.send_chain_key.zeroize();
        self.recv_chain_key.zeroize();
    }
}

impl RatchetState {
    /// Initialize the ratchet as the sender (Alice, who performed X3DH).
    ///
    /// The shared secret from X3DH seeds the root key. A DH ratchet step
    /// is performed immediately using the recipient's ratchet public key
    /// to derive the first send chain key.
    pub fn init_sender(
        shared_secret: &[u8; 32],
        their_ratchet_key: &X25519PublicKey,
    ) -> Result<Self, MessageError> {
        let our_ratchet = X25519KeyPair::generate();

        // Perform initial DH ratchet step.
        let dh_out = x25519_diffie_hellman(&our_ratchet.secret, their_ratchet_key);
        let (root_key, send_chain_key) = kdf_rk(shared_secret, &dh_out)?;

        Ok(Self {
            root_key,
            send_chain_key,
            recv_chain_key: [0u8; 32], // Set on first received message.
            send_ratchet_key: our_ratchet,
            recv_ratchet_pubkey: Some(their_ratchet_key.clone()),
            send_count: 0,
            recv_count: 0,
            previous_send_count: 0,
        })
    }

    /// Initialize the ratchet as the receiver (Bob, who responded to X3DH).
    ///
    /// Bob uses the shared secret as root key and his prekey keypair as
    /// the initial ratchet key. The first incoming message will trigger a
    /// DH ratchet step deriving the receive chain key.
    pub fn init_receiver(shared_secret: &[u8; 32], our_ratchet_key: X25519KeyPair) -> Self {
        Self {
            root_key: *shared_secret,
            send_chain_key: [0u8; 32],
            recv_chain_key: [0u8; 32],
            send_ratchet_key: our_ratchet_key,
            recv_ratchet_pubkey: None,
            send_count: 0,
            recv_count: 0,
            previous_send_count: 0,
        }
    }

    /// Reconstruct a `RatchetState` from its persisted components.
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        root_key: [u8; 32],
        send_chain_key: [u8; 32],
        recv_chain_key: [u8; 32],
        send_ratchet_secret: &[u8; 32],
        recv_ratchet_pubkey: Option<X25519PublicKey>,
        send_count: u32,
        recv_count: u32,
        previous_send_count: u32,
    ) -> Self {
        let send_ratchet_key = X25519KeyPair::from_secret_bytes(send_ratchet_secret);
        Self {
            root_key,
            send_chain_key,
            recv_chain_key,
            send_ratchet_key,
            recv_ratchet_pubkey,
            send_count,
            recv_count,
            previous_send_count,
        }
    }

    /// Return the current DH ratchet public key (included in message headers).
    pub fn public_ratchet_key(&self) -> &X25519PublicKey {
        &self.send_ratchet_key.public
    }

    /// Return the root key bytes (for persistence).
    pub fn root_key(&self) -> &[u8; 32] {
        &self.root_key
    }

    /// Return the send chain key bytes (for persistence).
    pub fn get_send_chain_key(&self) -> &[u8; 32] {
        &self.send_chain_key
    }

    /// Return the receive chain key bytes (for persistence).
    pub fn get_recv_chain_key(&self) -> &[u8; 32] {
        &self.recv_chain_key
    }

    /// Return the send ratchet secret key bytes (for persistence).
    pub fn send_ratchet_secret(&self) -> &[u8; 32] {
        self.send_ratchet_key.secret.as_bytes()
    }

    /// Return the receive ratchet public key (for persistence).
    pub fn get_recv_ratchet_pubkey(&self) -> Option<&X25519PublicKey> {
        self.recv_ratchet_pubkey.as_ref()
    }

    /// Return the current send message counter.
    pub fn get_send_count(&self) -> u32 {
        self.send_count
    }

    /// Return the current receive message counter.
    pub fn get_recv_count(&self) -> u32 {
        self.recv_count
    }

    /// Return the previous send chain length.
    pub fn get_previous_send_count(&self) -> u32 {
        self.previous_send_count
    }

    /// Encrypt a plaintext message, advancing the send chain.
    ///
    /// Returns the message header and ciphertext. The header must be
    /// sent alongside the ciphertext so the receiver can derive the
    /// decryption key.
    pub fn encrypt_message(
        &mut self,
        plaintext: &[u8],
    ) -> Result<(MessageHeader, Vec<u8>), MessageError> {
        // Derive per-message key from the send chain.
        let (new_chain_key, message_key) = kdf_ck(&self.send_chain_key)?;
        self.send_chain_key = new_chain_key;

        let header = MessageHeader {
            ratchet_key: self.send_ratchet_key.public.clone(),
            message_number: self.send_count,
            previous_chain_length: self.previous_send_count,
        };

        let ciphertext = seal(&message_key, plaintext).map_err(|e| MessageError::RatchetError {
            reason: format!("encryption failed: {e}"),
        })?;

        self.send_count += 1;

        Ok((header, ciphertext))
    }

    /// Decrypt a message, advancing the receive chain and performing
    /// a DH ratchet step if the sender's ratchet key has changed.
    pub fn decrypt_message(
        &mut self,
        header: &MessageHeader,
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, MessageError> {
        // Check if the sender's ratchet key has changed (DH ratchet step).
        let need_dh_step = match &self.recv_ratchet_pubkey {
            Some(existing) => existing != &header.ratchet_key,
            None => true,
        };

        if need_dh_step {
            self.dh_ratchet_step(&header.ratchet_key)?;
        }

        // Advance the receive chain to the correct message number.
        // Skip messages if necessary (out-of-order delivery).
        while self.recv_count < header.message_number {
            let (new_ck, _skipped_mk) = kdf_ck(&self.recv_chain_key)?;
            self.recv_chain_key = new_ck;
            self.recv_count += 1;
            // In a full implementation, skipped message keys would be stored.
        }

        // Derive the message key for this message.
        let (new_chain_key, message_key) = kdf_ck(&self.recv_chain_key)?;
        self.recv_chain_key = new_chain_key;
        self.recv_count += 1;

        open(&message_key, ciphertext).map_err(|e| MessageError::RatchetError {
            reason: format!("decryption failed: {e}"),
        })
    }

    /// Perform a DH ratchet step: rotate the root key using a new DH
    /// exchange with the peer's new ratchet public key.
    fn dh_ratchet_step(
        &mut self,
        their_new_ratchet_key: &X25519PublicKey,
    ) -> Result<(), MessageError> {
        self.previous_send_count = self.send_count;
        self.send_count = 0;
        self.recv_count = 0;
        self.recv_ratchet_pubkey = Some(their_new_ratchet_key.clone());

        // Derive new receive chain key from DH with their new key and our current key.
        let dh_recv = x25519_diffie_hellman(&self.send_ratchet_key.secret, their_new_ratchet_key);
        let (new_root, recv_chain_key) = kdf_rk(&self.root_key, &dh_recv)?;
        self.root_key = new_root;
        self.recv_chain_key = recv_chain_key;

        // Generate a new ratchet keypair for our next sending chain.
        self.send_ratchet_key = X25519KeyPair::generate();

        // Derive new send chain key from DH with their new key and our new key.
        let dh_send = x25519_diffie_hellman(&self.send_ratchet_key.secret, their_new_ratchet_key);
        let (new_root2, send_chain_key) = kdf_rk(&self.root_key, &dh_send)?;
        self.root_key = new_root2;
        self.send_chain_key = send_chain_key;

        Ok(())
    }
}

/// Root key KDF: derive a new root key and chain key from the current
/// root key and a DH output.
fn kdf_rk(root_key: &[u8; 32], dh_output: &[u8; 32]) -> Result<([u8; 32], [u8; 32]), MessageError> {
    // Concatenate root_key and dh_output as input key material.
    let mut ikm = [0u8; 64];
    ikm[..32].copy_from_slice(root_key);
    ikm[32..].copy_from_slice(dh_output);

    let new_root =
        hkdf_derive(&ikm, None, RATCHET_INFO).map_err(|e| MessageError::RatchetError {
            reason: format!("root KDF failed: {e}"),
        })?;

    // Derive chain key with a different info string.
    let chain_key =
        hkdf_derive(&ikm, None, CHAIN_INFO).map_err(|e| MessageError::RatchetError {
            reason: format!("chain KDF failed: {e}"),
        })?;

    ikm.zeroize();

    Ok((new_root, chain_key))
}

/// Chain key KDF: advance the chain key and derive a message key.
fn kdf_ck(chain_key: &[u8; 32]) -> Result<([u8; 32], [u8; 32]), MessageError> {
    let new_chain =
        hkdf_derive(chain_key, None, CHAIN_INFO).map_err(|e| MessageError::RatchetError {
            reason: format!("chain advance failed: {e}"),
        })?;

    let message_key =
        hkdf_derive(chain_key, None, MSG_KEY_INFO).map_err(|e| MessageError::RatchetError {
            reason: format!("message key derivation failed: {e}"),
        })?;

    Ok((new_chain, message_key))
}

#[cfg(test)]
#[path = "ratchet_tests.rs"]
mod tests;
