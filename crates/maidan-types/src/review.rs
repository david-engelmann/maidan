//! Required reviewers.
//!
//! A thread declares a **review requirement** — `required_count` (`k`) distinct
//! approvals — optionally from a **named reviewer set** (`n`). A reviewer
//! submits an [`ReviewDecision`] (approve / request-changes). The FSM
//! close-gate then refuses `closed` until `k` distinct **qualifying** approvals
//! exist — an approval qualifies when the reviewer is neither the thread's
//! `owner` nor its `assignee` (separation of duties) and, when a named set
//! exists, is in it — **and** no unresolved `refutes` edge blocks the thread.
//! This is a **gate**, not a poll/closer.
//!
//! A delivered `example.review.result/1` with any `critical` finding is fed in
//! as [`ReviewDecision::RequestChanges`] from a member who has
//! declared [`REVIEW_SKILL`]. That is a producer→reviewer adapter, not a new
//! gate: the close-gate still reads this table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{MemberId, ThreadId};

/// The member-skill tag a review agent declares. the adapter only writes
/// [`ReviewDecision::RequestChanges`] when the reviewer has this skill — so a
/// result from an implementer who is not review-skilled never arms the
/// close-gate.
pub const REVIEW_SKILL: &str = "review";

/// A reviewer's decision on a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReviewDecision {
    Approve,
    RequestChanges,
}

impl ReviewDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "approve" => Some(Self::Approve),
            "request_changes" => Some(Self::RequestChanges),
            _ => None,
        }
    }
}

/// A thread's review requirement: `required_count` distinct qualifying approvals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadReviewRequirement {
    pub thread_id: ThreadId,
    pub required_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A reviewer's decision record (one per `(thread, reviewer)` — re-submitting
/// changes it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadReview {
    pub thread_id: ThreadId,
    pub reviewer_id: MemberId,
    pub decision: ReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The delegate that actually submitted this review for `reviewer_id`, when
    /// one did. `None`: the reviewer submitted it itself. A delegate that owns
    /// or worked the thread is not counted, whoever it reviews as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<MemberId>,
    /// The evidence root of the review packet this verdict was given against.
    /// Only an approval bound to the thread's latest packet counts toward its
    /// close.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_root: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Set on an approval when a change request sent the thread back for
    /// rework: it approved a version that no longer stands, so it no longer
    /// counts. Re-submitting the review clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismissed_at: Option<DateTime<Utc>>,
}

/// One verdict in a thread's review history. [`ThreadReview`] holds each
/// reviewer's current decision, and re-submitting replaces it; every
/// submission is also appended here and never changed, so an owner can see
/// the sequence of verdicts that led to a land.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewVerdict {
    /// Position in the history; increases with every verdict.
    pub id: i64,
    pub thread_id: ThreadId,
    pub reviewer_id: MemberId,
    pub decision: ReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The delegate that submitted this verdict for `reviewer_id`, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<MemberId>,
    pub recorded_at: DateTime<Utc>,
}

/// What `submit_review` wrote: the reviewer's current review and the events
/// appended with it in one transaction, in log order, for the caller to
/// publish.
#[derive(Debug, Clone)]
pub struct ReviewSubmission {
    pub review: ThreadReview,
    /// The `ReviewSubmitted` event every verdict appends.
    pub submitted: crate::StoredEvent,
    /// The `ThreadStateChanged` (`in_review` → `open`) of a change request that
    /// sent the thread back. `None` when the verdict reopened nothing.
    pub reopened: Option<crate::StoredEvent>,
}

impl ReviewSubmission {
    /// Every event the verdict appended, oldest first.
    pub fn events(&self) -> impl Iterator<Item = &crate::StoredEvent> {
        std::iter::once(&self.submitted).chain(self.reopened.as_ref())
    }
}

/// The computed review standing of a thread — what the close-gate reads for the
/// **approval** side (the `refutes`-edge block is checked separately at the gate).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewStatus {
    /// Approvals required (0 when no requirement is set).
    pub required_count: i64,
    /// Distinct **qualifying** approvals: decision = approve, reviewer is neither
    /// owner nor assignee, and (when a named reviewer set exists) is in it.
    pub approvals: i64,
    /// Whether the approval requirement is met (`required_count == 0` or
    /// `approvals >= required_count`).
    pub approvals_met: bool,
}

/// The result a review packet pins: the hash of its canonical JSON and who
/// produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ResultEvidence {
    pub sha256: String,
    pub produced_by: MemberId,
}

/// The evidence a thread put in front of its reviewers when it went to review:
/// its result and its linked artifacts, each by content hash. Messages are the
/// conversation around the evidence, not the evidence, so a comment after an
/// approval does not undo it. No timestamps, so recomputing it from unchanged
/// evidence gives the same root on either backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EvidenceManifest {
    pub thread_id: ThreadId,
    pub result: Option<ResultEvidence>,
    /// Linked artifacts' sha256 hashes, sorted.
    pub artifacts: Vec<String>,
    /// How far each piece of evidence can be trusted, judged once at the
    /// hand-off and part of the root, so an approval names the tiers it was
    /// shown as well as the evidence. Empty on packets recorded before tiers
    /// existed, which serialize (and so hash) exactly as they did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attestations: Vec<EvidenceAttestation>,
}

/// How far a piece of evidence can be trusted, from strongest to weakest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AttestationTier {
    /// Checked by someone independent of the work: a qualifying land-gate pass.
    Verified,
    /// Put in front of the reviewers by a member who never held the thread.
    Attached,
    /// The work's own account of itself: a worker's result, or an artifact a
    /// worker linked.
    SelfReported,
}

impl AttestationTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Attached => "attached",
            Self::SelfReported => "self_reported",
        }
    }
}

/// What kind of evidence an attestation is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// The thread's result; `sha256` is the result's hash.
    Result,
    /// A linked artifact; `sha256` is its hash.
    Artifact,
    /// A land-gate pass standing at the hand-off; `sha256` is the artifact
    /// the pass names, when it names one.
    LandGate,
}

/// One piece of evidence and the tier it was given at the hand-off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EvidenceAttestation {
    pub kind: EvidenceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub tier: AttestationTier,
    /// Who put it there: the result's producer, the artifact's linker, or the
    /// pass's recorder.
    pub attested_by: MemberId,
}

/// Whether an approval of this evidence would rest on self-reported evidence
/// only: there is evidence, and none of it is better than self-reported.
/// Packets without tiers never warn, because nothing was judged.
pub fn self_reported_only(attestations: &[EvidenceAttestation]) -> bool {
    !attestations.is_empty()
        && attestations
            .iter()
            .all(|a| a.tier == AttestationTier::SelfReported)
}

impl EvidenceManifest {
    /// The evidence root: the sha256 of the manifest's canonical JSON (keys
    /// sorted, no whitespace). A decision that names it names exactly this
    /// evidence.
    pub fn root(&self) -> Result<String, crate::signed_export::SignedExportError> {
        let value = serde_json::to_value(self)
            .map_err(|e| crate::signed_export::SignedExportError::Json(e.to_string()))?;
        Ok(sha256_hex(&crate::signed_export::canonical_json(&value)?))
    }

    /// Whether `other` holds the same evidence: the same result and the same
    /// linked artifacts. Tiers are left out on purpose. They were judged at
    /// the hand-off, and re-judging them later would let a change in who
    /// worked the thread move them without the evidence moving.
    pub fn same_evidence(&self, other: &EvidenceManifest) -> bool {
        self.thread_id == other.thread_id
            && self.result == other.result
            && self.artifacts == other.artifacts
    }
}

/// The sha256 of a result's canonical JSON, so the same result hashes the same
/// however a backend ordered its keys.
pub fn result_sha256(
    result: &serde_json::Value,
) -> Result<String, crate::signed_export::SignedExportError> {
    Ok(sha256_hex(&crate::signed_export::canonical_json(result)?))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// An immutable record of one hand-off to review: who started the review, the
/// manifest of what it was handed, and that manifest's root. Each
/// `start_review` writes one; nothing updates it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewPacket {
    pub id: uuid::Uuid,
    pub thread_id: ThreadId,
    pub requested_by: MemberId,
    /// The thread's version at the hand-off: how many writes its content had
    /// seen, messages included. A later version means the thread moved on.
    pub thread_version: i64,
    pub manifest: EvidenceManifest,
    pub evidence_root: String,
    /// The server's warning: approving this packet would rest on
    /// self-reported evidence only. Derived from `manifest.attestations`.
    pub self_reported_only: bool,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod packet_tests {
    use super::*;

    fn manifest() -> EvidenceManifest {
        EvidenceManifest {
            thread_id: ThreadId(uuid::Uuid::nil()),
            result: Some(ResultEvidence {
                sha256: result_sha256(&serde_json::json!({"b": 1, "a": [true, null]})).unwrap(),
                produced_by: MemberId(uuid::Uuid::nil()),
            }),
            artifacts: vec!["aa".repeat(32)],
            attestations: Vec::new(),
        }
    }

    #[test]
    fn a_result_hashes_the_same_whatever_its_key_order() {
        let ordered = serde_json::json!({"a": [true, null], "b": 1});
        let shuffled: serde_json::Value =
            serde_json::from_str(r#"{"b":1,"a":[true,null]}"#).unwrap();
        assert_eq!(
            result_sha256(&ordered).unwrap(),
            result_sha256(&shuffled).unwrap()
        );
    }

    #[test]
    fn the_root_moves_with_any_part_of_the_manifest() {
        let base = manifest();
        let root = base.root().unwrap();
        assert_eq!(root.len(), 64);
        assert_eq!(root, manifest().root().unwrap(), "deterministic");
        let mut other_result = manifest();
        other_result.result = None;
        let mut more = manifest();
        more.artifacts.push("bb".repeat(32));
        for changed in [other_result, more] {
            assert_ne!(changed.root().unwrap(), root, "{changed:?}");
        }
    }
}
