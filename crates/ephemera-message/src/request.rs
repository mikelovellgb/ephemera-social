//! Message requests from strangers.
//!
//! When a non-connected pseudonym wants to initiate a conversation, they
//! must first send a `MessageRequest`. The recipient can accept, reject,
//! or ignore it. Accepting opens the DM channel without creating a
//! mutual connection.

use ephemera_types::{IdentityKey, Timestamp};
use serde::{Deserialize, Serialize};

use crate::MessageError;

/// Maximum length of the introductory message in a request (characters).
const MAX_INTRO_LENGTH: usize = 280;

/// The status of a message request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRequestStatus {
    /// Waiting for the recipient to act.
    Pending,
    /// The recipient accepted; DM channel is open.
    Accepted,
    /// The recipient rejected; no further messages allowed.
    Rejected,
}

/// A request from a stranger to start a direct messaging conversation.
///
/// The sender must have completed proof-of-work at full difficulty (~30s)
/// before this request can be submitted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRequest {
    /// The identity requesting to message.
    pub sender: IdentityKey,
    /// The intended recipient.
    pub recipient: IdentityKey,
    /// Optional introductory message (max 280 characters).
    pub intro_message: Option<String>,
    /// When the request was created.
    pub created_at: Timestamp,
    /// Current status of the request.
    pub status: MessageRequestStatus,
}

impl MessageRequest {
    /// Create a new pending message request.
    ///
    /// Returns an error if the intro message exceeds the character limit.
    pub fn new(
        sender: IdentityKey,
        recipient: IdentityKey,
        intro_message: Option<String>,
    ) -> Result<Self, MessageError> {
        if let Some(ref msg) = intro_message {
            if msg.chars().count() > MAX_INTRO_LENGTH {
                return Err(MessageError::BodyTooLarge {
                    got: msg.chars().count(),
                    max: MAX_INTRO_LENGTH,
                });
            }
        }
        Ok(Self {
            sender,
            recipient,
            intro_message,
            created_at: Timestamp::now(),
            status: MessageRequestStatus::Pending,
        })
    }

    /// Accept the request, transitioning to `Accepted` status.
    pub fn accept(&mut self) -> Result<(), MessageError> {
        if self.status != MessageRequestStatus::Pending {
            return Err(MessageError::InvalidRequestState {
                expected: "Pending".into(),
                got: format!("{:?}", self.status),
            });
        }
        self.status = MessageRequestStatus::Accepted;
        Ok(())
    }

    /// Reject the request, transitioning to `Rejected` status.
    pub fn reject(&mut self) -> Result<(), MessageError> {
        if self.status != MessageRequestStatus::Pending {
            return Err(MessageError::InvalidRequestState {
                expected: "Pending".into(),
                got: format!("{:?}", self.status),
            });
        }
        self.status = MessageRequestStatus::Rejected;
        Ok(())
    }

    /// Whether the request has been accepted.
    pub fn is_accepted(&self) -> bool {
        self.status == MessageRequestStatus::Accepted
    }

    /// Whether the request is still pending.
    pub fn is_pending(&self) -> bool {
        self.status == MessageRequestStatus::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> IdentityKey {
        IdentityKey::from_bytes([1; 32])
    }

    fn bob() -> IdentityKey {
        IdentityKey::from_bytes([2; 32])
    }

    #[test]
    fn create_request_without_intro() {
        let req = MessageRequest::new(alice(), bob(), None).unwrap();
        assert!(req.is_pending());
        assert!(req.intro_message.is_none());
    }

    #[test]
    fn create_request_with_intro() {
        let req = MessageRequest::new(alice(), bob(), Some("Hello!".into())).unwrap();
        assert_eq!(req.intro_message.as_deref(), Some("Hello!"));
    }

    #[test]
    fn intro_too_long_rejected() {
        let long_msg = "a".repeat(MAX_INTRO_LENGTH + 1);
        let result = MessageRequest::new(alice(), bob(), Some(long_msg));
        assert!(result.is_err());
    }

    #[test]
    fn accept_request() {
        let mut req = MessageRequest::new(alice(), bob(), None).unwrap();
        assert!(req.accept().is_ok());
        assert!(req.is_accepted());
    }

    #[test]
    fn reject_request() {
        let mut req = MessageRequest::new(alice(), bob(), None).unwrap();
        assert!(req.reject().is_ok());
        assert_eq!(req.status, MessageRequestStatus::Rejected);
    }

    #[test]
    fn cannot_accept_already_rejected() {
        let mut req = MessageRequest::new(alice(), bob(), None).unwrap();
        req.reject().unwrap();
        assert!(req.accept().is_err());
    }

    #[test]
    fn cannot_reject_already_accepted() {
        let mut req = MessageRequest::new(alice(), bob(), None).unwrap();
        req.accept().unwrap();
        assert!(req.reject().is_err());
    }
}
