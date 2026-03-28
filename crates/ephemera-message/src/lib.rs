#![allow(clippy::duplicate_mod)]
//! Private messaging for the Ephemera decentralized social platform.
//!
//! Provides direct messaging with end-to-end encryption using the Signal
//! Protocol-style X3DH key exchange and Double Ratchet for forward secrecy
//! and post-compromise security. Message delivery uses sealed-sender
//! envelopes that hide the sender's identity from relay nodes.

mod conversation;
pub mod dead_drop;
pub mod encryption;
mod error;
mod message;
pub mod prekey;
pub mod ratchet;
mod request;
mod sealed;
mod service;
pub mod session;
pub mod x3dh;

pub use conversation::{Conversation, ConversationList};
pub use dead_drop::{DeadDropEnvelope, DeadDropService};
pub use encryption::MessageEncryption;
pub use error::MessageError;
pub use message::{DirectMessage, MessageId, MessageStatus};
pub use prekey::{generate_prekey_bundle, validate_prekey_bundle, PrekeyBundle};
pub use ratchet::{MessageHeader, RatchetState};
pub use request::{MessageRequest, MessageRequestStatus};
pub use sealed::{SealedEnvelope, SealedPayload};
pub use service::{ConversationSummary, MessageService, StoredMessage};
pub use session::{EncryptedMessage, SessionManager};
pub use x3dh::{x3dh_initiate, x3dh_respond, X3dhInitialMessage, X3dhInitiatorResult};

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod protocol_tests;
