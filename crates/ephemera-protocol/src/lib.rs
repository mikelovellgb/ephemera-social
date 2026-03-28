//! Wire protocol definitions for the Ephemera P2P network.
//!
//! Defines the envelope structure, message types, encoding/decoding, and
//! protocol version negotiation. All messages on the wire are wrapped in a
//! [`ProtocolEnvelope`] and serialized via the [`codec`] module.

pub mod codec;
pub mod envelope;
pub mod messages;
pub mod version;

pub use envelope::ProtocolEnvelope;
pub use messages::MessageType;
pub use version::ProtocolVersion;
