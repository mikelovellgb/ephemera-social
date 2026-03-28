//! NAT traversal helpers.
//!
//! Iroh handles most NAT traversal automatically via its relay-based
//! hole-punching mechanism. This module provides helper types for
//! tracking NAT status and aiding the connection upgrade path from
//! relayed to direct.

use std::net::SocketAddr;
use std::time::Instant;

/// Detected NAT type for this node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatType {
    /// No NAT detected -- directly reachable.
    Open,
    /// Full-cone NAT -- any external host can reach the mapped port.
    FullCone,
    /// Restricted-cone NAT -- only hosts we have sent to can reach us.
    RestrictedCone,
    /// Port-restricted-cone NAT -- host + port must match.
    PortRestricted,
    /// Symmetric NAT -- different mapping for each destination (hardest).
    Symmetric,
    /// NAT type is unknown (detection not yet performed).
    Unknown,
}

impl NatType {
    /// Whether hole-punching is likely to succeed with this NAT type.
    #[must_use]
    pub fn hole_punch_likely(&self) -> bool {
        matches!(
            self,
            NatType::Open | NatType::FullCone | NatType::RestrictedCone | NatType::PortRestricted
        )
    }

    /// Whether we can accept inbound connections without relay assistance.
    #[must_use]
    pub fn supports_inbound(&self) -> bool {
        matches!(self, NatType::Open | NatType::FullCone)
    }
}

/// NAT traversal status tracker for a connection.
#[derive(Debug)]
pub struct NatStatus {
    /// Detected NAT type.
    pub nat_type: NatType,
    /// Our externally-visible address (as reported by STUN or relay).
    pub external_addr: Option<SocketAddr>,
    /// When the NAT type was last checked.
    pub last_check: Option<Instant>,
    /// Whether we are currently behind a relay.
    pub using_relay: bool,
}

impl NatStatus {
    /// Create a new unknown NAT status.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            nat_type: NatType::Unknown,
            external_addr: None,
            last_check: None,
            using_relay: false,
        }
    }

    /// Update the NAT type after detection.
    pub fn set_nat_type(&mut self, nat_type: NatType) {
        self.nat_type = nat_type;
        self.last_check = Some(Instant::now());
    }

    /// Update the externally-visible address.
    pub fn set_external_addr(&mut self, addr: SocketAddr) {
        self.external_addr = Some(addr);
    }

    /// Whether a direct connection upgrade should be attempted.
    ///
    /// We should try to upgrade from relay to direct if:
    /// 1. We are currently on a relay
    /// 2. Our NAT type supports hole-punching
    #[must_use]
    pub fn should_attempt_upgrade(&self) -> bool {
        self.using_relay && self.nat_type.hole_punch_likely()
    }
}

impl Default for NatStatus {
    fn default() -> Self {
        Self::unknown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nat_type_hole_punch() {
        assert!(NatType::Open.hole_punch_likely());
        assert!(NatType::FullCone.hole_punch_likely());
        assert!(NatType::RestrictedCone.hole_punch_likely());
        assert!(!NatType::Symmetric.hole_punch_likely());
        assert!(!NatType::Unknown.hole_punch_likely());
    }

    #[test]
    fn nat_type_inbound() {
        assert!(NatType::Open.supports_inbound());
        assert!(NatType::FullCone.supports_inbound());
        assert!(!NatType::Symmetric.supports_inbound());
    }

    #[test]
    fn upgrade_logic() {
        let mut status = NatStatus::unknown();
        status.using_relay = true;
        status.set_nat_type(NatType::FullCone);
        assert!(status.should_attempt_upgrade());

        status.set_nat_type(NatType::Symmetric);
        assert!(!status.should_attempt_upgrade());

        status.using_relay = false;
        status.set_nat_type(NatType::FullCone);
        assert!(!status.should_attempt_upgrade());
    }
}
