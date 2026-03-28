//! Network-related types.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Current state of the node's network connectivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkStatus {
    /// Not connected to any peers.
    Disconnected,
    /// Establishing connections.
    Connecting,
    /// Connected and participating in the network.
    Connected,
    /// Connected but behind a restrictive NAT / firewall.
    Limited,
}

impl fmt::Display for NetworkStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => write!(f, "disconnected"),
            Self::Connecting => write!(f, "connecting"),
            Self::Connected => write!(f, "connected"),
            Self::Limited => write!(f, "limited"),
        }
    }
}
