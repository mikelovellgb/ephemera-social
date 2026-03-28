//! Content moderation subsystem for the Ephemera platform.
//!
//! Provides user-facing reporting, hash-based content blocklisting, and
//! content filtering. Moderation operates without breaking encryption or
//! deanonymizing users -- the system relies on client-side hash checks,
//! community reports, and reputation consequences.

mod blocklist;
mod error;
mod filter;
mod report;

pub use blocklist::LocalBlocklist;
pub use error::ModerationError;
pub use filter::{ContentFilter, FilterResult};
pub use report::{Report, ReportReason, ReportService};
