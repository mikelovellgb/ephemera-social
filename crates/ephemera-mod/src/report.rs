//! Content reporting.
//!
//! Users can report content they find abusive. Reports are stored locally
//! and (post-MVP) submitted anonymously to the moderation quorum via
//! gossip. The reporter's identity is never revealed to the reported user.

use ephemera_types::{ContentId, IdentityKey, Timestamp};
use serde::{Deserialize, Serialize};

use crate::ModerationError;

/// Maximum length of an optional report description.
const MAX_DESCRIPTION_LENGTH: usize = 280;

/// Why the content was reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportReason {
    /// Targeted harassment of a user.
    Harassment,
    /// Hate speech targeting a protected group.
    HateSpeech,
    /// Graphic violence or threats.
    Violence,
    /// Unsolicited commercial messages or repetitive content.
    Spam,
    /// Suspected child sexual abuse material (escalated to automated handling).
    Csam,
    /// Pretending to be another user.
    Impersonation,
    /// Other reason with a user-provided description.
    Other(String),
}

/// A content report submitted by a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// The identity of the reporter.
    pub reporter: IdentityKey,
    /// The content hash of the reported item.
    pub content_id: ContentId,
    /// The reason for the report.
    pub reason: ReportReason,
    /// Optional additional description.
    pub description: Option<String>,
    /// When the report was created.
    pub timestamp: Timestamp,
}

/// Service for managing content reports.
///
/// For the PoC this stores reports in memory. Production will persist
/// to SQLite and forward to the moderation quorum.
pub struct ReportService {
    reports: Vec<Report>,
}

impl ReportService {
    /// Create an empty report service.
    pub fn new() -> Self {
        Self {
            reports: Vec::new(),
        }
    }

    /// Submit a new report.
    ///
    /// Returns an error if the description is too long or a report from
    /// the same reporter for the same content already exists.
    pub fn create_report(
        &mut self,
        reporter: IdentityKey,
        content_id: ContentId,
        reason: ReportReason,
        description: Option<String>,
    ) -> Result<&Report, ModerationError> {
        // Validate description length
        if let Some(ref desc) = description {
            if desc.chars().count() > MAX_DESCRIPTION_LENGTH {
                return Err(ModerationError::DescriptionTooLong {
                    got: desc.chars().count(),
                    max: MAX_DESCRIPTION_LENGTH,
                });
            }
        }

        // Check for duplicate
        let already_reported = self
            .reports
            .iter()
            .any(|r| r.reporter == reporter && r.content_id == content_id);
        if already_reported {
            return Err(ModerationError::DuplicateReport {
                content_id: content_id.to_string(),
            });
        }

        let report = Report {
            reporter,
            content_id,
            reason,
            description,
            timestamp: Timestamp::now(),
        };
        self.reports.push(report);
        Ok(self.reports.last().expect("just pushed"))
    }

    /// List all reports, most recent first.
    pub fn list_reports(&self) -> &[Report] {
        &self.reports
    }

    /// List reports for a specific content hash.
    pub fn reports_for_content(&self, content_id: &ContentId) -> Vec<&Report> {
        self.reports
            .iter()
            .filter(|r| r.content_id == *content_id)
            .collect()
    }

    /// Count how many distinct reporters flagged a piece of content.
    pub fn reporter_count(&self, content_id: &ContentId) -> usize {
        self.reports
            .iter()
            .filter(|r| r.content_id == *content_id)
            .map(|r| &r.reporter)
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    /// Total number of reports.
    pub fn len(&self) -> usize {
        self.reports.len()
    }

    /// Whether no reports have been filed.
    pub fn is_empty(&self) -> bool {
        self.reports.is_empty()
    }
}

impl Default for ReportService {
    fn default() -> Self {
        Self::new()
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

    fn content_hash(n: u8) -> ContentId {
        ContentId::from_digest([n; 32])
    }

    #[test]
    fn create_report_success() {
        let mut svc = ReportService::new();
        let result = svc.create_report(alice(), content_hash(1), ReportReason::Spam, None);
        assert!(result.is_ok());
        assert_eq!(svc.len(), 1);
    }

    #[test]
    fn create_report_with_description() {
        let mut svc = ReportService::new();
        let result = svc.create_report(
            alice(),
            content_hash(1),
            ReportReason::Other("suspicious links".into()),
            Some("Contains phishing URL".into()),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn description_too_long_rejected() {
        let mut svc = ReportService::new();
        let long_desc = "a".repeat(MAX_DESCRIPTION_LENGTH + 1);
        let result = svc.create_report(
            alice(),
            content_hash(1),
            ReportReason::Spam,
            Some(long_desc),
        );
        assert!(result.is_err());
    }

    #[test]
    fn duplicate_report_rejected() {
        let mut svc = ReportService::new();
        svc.create_report(alice(), content_hash(1), ReportReason::Spam, None)
            .unwrap();
        let result = svc.create_report(alice(), content_hash(1), ReportReason::Harassment, None);
        assert!(result.is_err());
    }

    #[test]
    fn different_reporters_allowed() {
        let mut svc = ReportService::new();
        svc.create_report(alice(), content_hash(1), ReportReason::Spam, None)
            .unwrap();
        let result = svc.create_report(bob(), content_hash(1), ReportReason::Spam, None);
        assert!(result.is_ok());
        assert_eq!(svc.reporter_count(&content_hash(1)), 2);
    }

    #[test]
    fn reports_for_content() {
        let mut svc = ReportService::new();
        svc.create_report(alice(), content_hash(1), ReportReason::Spam, None)
            .unwrap();
        svc.create_report(bob(), content_hash(1), ReportReason::Spam, None)
            .unwrap();
        svc.create_report(alice(), content_hash(2), ReportReason::Violence, None)
            .unwrap();

        let reports = svc.reports_for_content(&content_hash(1));
        assert_eq!(reports.len(), 2);
    }

    #[test]
    fn list_reports_returns_all() {
        let mut svc = ReportService::new();
        svc.create_report(alice(), content_hash(1), ReportReason::Spam, None)
            .unwrap();
        svc.create_report(bob(), content_hash(2), ReportReason::Violence, None)
            .unwrap();

        assert_eq!(svc.list_reports().len(), 2);
    }
}
