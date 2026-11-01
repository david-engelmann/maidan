//! Evidence records: what an agent or a person attaches to a task to show the
//! work happened. A test run, a CI run, a diff, a review verdict given
//! somewhere else, a screenshot by URL, or a plain note. Each says what kind
//! of evidence it is, where it came from, who recorded it and when, and points
//! at it by URL, by content digest, or both.
//!
//! Records are never edited or removed. They are read on the thread's
//! evidence timeline next to the evidence the server pins at a review
//! hand-off (the result, linked artifacts, land-gate verdicts), but they are
//! not pinned themselves: an approval's evidence root does not move when a
//! record is added.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ids::{MemberId, ThreadId};

/// The longest `source` accepted, in characters.
pub const EVIDENCE_SOURCE_MAX: usize = 200;
/// The longest `url` accepted, in bytes.
pub const EVIDENCE_URL_MAX: usize = 2048;
/// The longest `note` accepted, in characters.
pub const EVIDENCE_NOTE_MAX: usize = 4000;

/// What a record is evidence of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRecordKind {
    /// A test run: a suite's outcome, by its log or report.
    TestRun,
    /// A CI run, by its run page.
    CiRun,
    /// A diff or a change, by its page or its digest.
    Diff,
    /// A verdict given outside Maidan, such as a pull-request review.
    ReviewVerdict,
    /// A screenshot, by URL.
    Screenshot,
    /// A note in words.
    Note,
}

impl EvidenceRecordKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TestRun => "test_run",
            Self::CiRun => "ci_run",
            Self::Diff => "diff",
            Self::ReviewVerdict => "review_verdict",
            Self::Screenshot => "screenshot",
            Self::Note => "note",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "test_run" => Some(Self::TestRun),
            "ci_run" => Some(Self::CiRun),
            "diff" => Some(Self::Diff),
            "review_verdict" => Some(Self::ReviewVerdict),
            "screenshot" => Some(Self::Screenshot),
            "note" => Some(Self::Note),
            _ => None,
        }
    }
}

/// One evidence record on a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EvidenceRecord {
    pub id: Uuid,
    pub thread_id: ThreadId,
    pub kind: EvidenceRecordKind,
    /// Where the evidence came from, such as `github-actions`, `cargo test`
    /// or `pr-review`. Free text, as the recorder names it.
    pub source: String,
    /// Where to look at it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The sha256 of its content, lowercase hex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// What it shows, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The member whose token recorded it.
    pub recorded_by: MemberId,
    pub recorded_at: DateTime<Utc>,
}

/// A record to add: `POST /threads/{id}/evidence-records` and the
/// `add_thread_evidence` tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct NewEvidenceRecord {
    pub kind: EvidenceRecordKind,
    /// Where the evidence came from, 1 to 200 characters.
    pub source: String,
    /// An `http` or `https` URL, at most 2048 bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// A sha256, 64 hex characters; stored lowercase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// At most 4000 characters. Required for a `note`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl NewEvidenceRecord {
    /// The record in its one stored form, or why it is refused. A record must
    /// point at something: a URL, a digest, or a note. A screenshot needs a
    /// URL and a note needs words.
    pub fn normalized(self) -> Result<Self, String> {
        let source = self.source.trim().to_string();
        if source.is_empty() || source.chars().count() > EVIDENCE_SOURCE_MAX {
            return Err(format!(
                "source must be 1 to {EVIDENCE_SOURCE_MAX} characters"
            ));
        }
        let url = match self.url.map(|u| u.trim().to_string()) {
            Some(u) if u.is_empty() => None,
            Some(u) => {
                if u.len() > EVIDENCE_URL_MAX {
                    return Err(format!("url must be at most {EVIDENCE_URL_MAX} bytes"));
                }
                let lower = u.to_ascii_lowercase();
                let rest = lower
                    .strip_prefix("https://")
                    .or_else(|| lower.strip_prefix("http://"));
                match rest {
                    Some(host) if !host.is_empty() && !host.starts_with('/') => {}
                    _ => return Err("url must be an http or https URL".into()),
                }
                if u.chars().any(char::is_whitespace) {
                    return Err("url must not contain whitespace".into());
                }
                Some(u)
            }
            None => None,
        };
        let sha256 = match self.sha256.map(|s| s.trim().to_ascii_lowercase()) {
            Some(s) if s.is_empty() => None,
            Some(s) => {
                if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("sha256 must be 64 hex characters".into());
                }
                Some(s)
            }
            None => None,
        };
        let note = match self.note.map(|n| n.trim().to_string()) {
            Some(n) if n.is_empty() => None,
            Some(n) => {
                if n.chars().count() > EVIDENCE_NOTE_MAX {
                    return Err(format!("note must be at most {EVIDENCE_NOTE_MAX} characters"));
                }
                Some(n)
            }
            None => None,
        };
        if url.is_none() && sha256.is_none() && note.is_none() {
            return Err("a record needs a url, a sha256 or a note".into());
        }
        if self.kind == EvidenceRecordKind::Screenshot && url.is_none() {
            return Err("a screenshot needs a url".into());
        }
        if self.kind == EvidenceRecordKind::Note && note.is_none() {
            return Err("a note needs a note".into());
        }
        Ok(Self {
            kind: self.kind,
            source,
            url,
            sha256,
            note,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(kind: EvidenceRecordKind) -> NewEvidenceRecord {
        NewEvidenceRecord {
            kind,
            source: " github-actions ".into(),
            url: None,
            sha256: None,
            note: None,
        }
    }

    #[test]
    fn every_kind_round_trips() {
        for kind in [
            EvidenceRecordKind::TestRun,
            EvidenceRecordKind::CiRun,
            EvidenceRecordKind::Diff,
            EvidenceRecordKind::ReviewVerdict,
            EvidenceRecordKind::Screenshot,
            EvidenceRecordKind::Note,
        ] {
            assert_eq!(EvidenceRecordKind::parse(kind.as_str()), Some(kind));
            let wire = serde_json::to_value(kind).unwrap_or_default();
            assert_eq!(wire.as_str(), Some(kind.as_str()));
        }
        assert_eq!(EvidenceRecordKind::parse("video"), None);
    }

    #[test]
    fn a_record_is_trimmed_and_its_digest_lowercased() {
        let mut r = new(EvidenceRecordKind::CiRun);
        r.url = Some(" https://ci.example.com/runs/1 ".into());
        r.sha256 = Some("AB".repeat(32));
        let n = r.normalized().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(n.source, "github-actions");
        assert_eq!(n.url.as_deref(), Some("https://ci.example.com/runs/1"));
        assert_eq!(n.sha256, Some("ab".repeat(32)));
    }

    #[test]
    fn a_record_that_points_at_nothing_is_refused() {
        assert!(new(EvidenceRecordKind::TestRun).normalized().is_err());
        let mut blank = new(EvidenceRecordKind::Diff);
        blank.url = Some("  ".into());
        assert!(blank.normalized().is_err());
    }

    #[test]
    fn bad_urls_digests_and_sources_are_refused() {
        for url in ["ftp://x", "javascript:alert(1)", "https://", "https:///x", "https://a b"] {
            let mut r = new(EvidenceRecordKind::CiRun);
            r.url = Some(url.into());
            assert!(r.normalized().is_err(), "{url}");
        }
        let mut r = new(EvidenceRecordKind::Diff);
        r.sha256 = Some("xyz".into());
        assert!(r.normalized().is_err());
        let mut r = new(EvidenceRecordKind::Note);
        r.source = " ".into();
        r.note = Some("ok".into());
        assert!(r.normalized().is_err());
    }

    #[test]
    fn a_screenshot_needs_a_url_and_a_note_needs_words() {
        let mut shot = new(EvidenceRecordKind::Screenshot);
        shot.note = Some("the board".into());
        assert!(shot.normalized().is_err());
        let mut note = new(EvidenceRecordKind::Note);
        note.url = Some("https://example.com".into());
        assert!(note.normalized().is_err());
        note.note = Some("checked by hand".into());
        assert!(note.normalized().is_ok());
    }

    #[test]
    fn unknown_fields_are_refused() {
        let parsed: Result<NewEvidenceRecord, _> = serde_json::from_value(serde_json::json!({
            "kind": "note", "source": "me", "note": "x", "extra": 1
        }));
        assert!(parsed.is_err());
    }
}
