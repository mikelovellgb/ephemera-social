//! Protocol version constants and compatibility checking.
//!
//! Ephemera uses a simple major.minor versioning scheme for the wire protocol.
//! Nodes with the same major version are compatible; minor versions indicate
//! backward-compatible additions.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Current protocol major version.
pub const PROTOCOL_MAJOR: u16 = 1;

/// Current protocol minor version.
pub const PROTOCOL_MINOR: u16 = 0;

/// Protocol version carried in every envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProtocolVersion {
    /// Major version -- breaking changes increment this.
    pub major: u16,
    /// Minor version -- backward-compatible additions.
    pub minor: u16,
}

impl ProtocolVersion {
    /// The current protocol version compiled into this binary.
    #[must_use]
    pub fn current() -> Self {
        Self {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
        }
    }

    /// Create a specific version.
    #[must_use]
    pub fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Check whether `other` is compatible with this version.
    ///
    /// Two versions are compatible if they share the same major version
    /// and the remote minor is not ahead of ours (we can't understand
    /// features we don't know about).
    #[must_use]
    pub fn is_compatible_with(&self, other: &ProtocolVersion) -> bool {
        self.major == other.major
    }

    /// Check whether `other` is strictly identical to this version.
    #[must_use]
    pub fn is_exact_match(&self, other: &ProtocolVersion) -> bool {
        self.major == other.major && self.minor == other.minor
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}.{}", self.major, self.minor)
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        Self::current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version() {
        let v = ProtocolVersion::current();
        assert_eq!(v.major, PROTOCOL_MAJOR);
        assert_eq!(v.minor, PROTOCOL_MINOR);
    }

    #[test]
    fn same_major_is_compatible() {
        let a = ProtocolVersion::new(1, 0);
        let b = ProtocolVersion::new(1, 3);
        assert!(a.is_compatible_with(&b));
        assert!(b.is_compatible_with(&a));
    }

    #[test]
    fn different_major_not_compatible() {
        let a = ProtocolVersion::new(1, 0);
        let b = ProtocolVersion::new(2, 0);
        assert!(!a.is_compatible_with(&b));
    }

    #[test]
    fn exact_match() {
        let a = ProtocolVersion::new(1, 0);
        let b = ProtocolVersion::new(1, 0);
        let c = ProtocolVersion::new(1, 1);
        assert!(a.is_exact_match(&b));
        assert!(!a.is_exact_match(&c));
    }

    #[test]
    fn display() {
        let v = ProtocolVersion::new(1, 2);
        assert_eq!(v.to_string(), "v1.2");
    }
}
