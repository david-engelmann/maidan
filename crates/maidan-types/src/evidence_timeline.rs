//! A thread's evidence timeline: everything the server recorded as evidence
//! for one task, in the order it happened.
//!
//! The pieces already existed as separate reads (linked artifacts, the
//! result, land-gate history, review packets, review history), and a person
//! deciding on a task had to stitch them together to see what was shown to
//! whom, and whether the evidence moved after the review was handed it. This
//! only reads and orders what was recorded: nothing here judges a gate or a
//! decision.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{MemberId, ThreadId};
use crate::land_gate::LandGateVerdict;
use crate::models::{ThreadArtifact, ThreadResult};
use crate::review::{result_sha256, EvidenceManifest, ResultEvidence, ReviewPacket, ReviewVerdict};

/// One recorded piece of evidence, or one step of its review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceTimelineEntry {
    /// An artifact was linked to the thread.
    ArtifactLinked {
        at: DateTime<Utc>,
        artifact: ThreadArtifact,
    },
    /// The thread's current result was recorded. A result is overwritten
    /// when re-set, so only the standing one is on the timeline; a review
    /// packet keeps the hash of the one it was handed.
    ResultRecorded {
        at: DateTime<Utc>,
        /// The hash a review packet pins the result by; `None` only if the
        /// stored result could not be canonicalized.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
        produced_by: MemberId,
    },
    /// A land-gate verdict was recorded.
    LandGateVerdict {
        at: DateTime<Utc>,
        verdict: LandGateVerdict,
    },
    /// The thread was handed to review with this packet.
    ReviewRequested {
        at: DateTime<Utc>,
        packet: ReviewPacket,
    },
    /// A reviewer gave a verdict.
    ReviewVerdict {
        at: DateTime<Utc>,
        verdict: ReviewVerdict,
    },
}

impl EvidenceTimelineEntry {
    pub fn at(&self) -> DateTime<Utc> {
        match self {
            Self::ArtifactLinked { at, .. }
            | Self::ResultRecorded { at, .. }
            | Self::LandGateVerdict { at, .. }
            | Self::ReviewRequested { at, .. }
            | Self::ReviewVerdict { at, .. } => *at,
        }
    }

    /// Order among entries recorded at the same instant: evidence before
    /// the hand-off that pins it, and the hand-off before verdicts on it.
    fn rank(&self) -> u8 {
        match self {
            Self::ArtifactLinked { .. } => 0,
            Self::ResultRecorded { .. } => 1,
            Self::LandGateVerdict { .. } => 2,
            Self::ReviewRequested { .. } => 3,
            Self::ReviewVerdict { .. } => 4,
        }
    }
}

/// A thread's evidence, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EvidenceTimeline {
    pub thread_id: ThreadId,
    /// The thread's version now; each `review_requested` entry carries the
    /// version it was handed at.
    pub thread_version: i64,
    /// Whether the result or the linked artifacts differ from what the
    /// latest review packet was handed. `None` before any hand-off.
    /// Messages are not evidence, so a comment does not set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_changed_since_review: Option<bool>,
    pub entries: Vec<EvidenceTimelineEntry>,
}

/// What the timeline is assembled from: the thread's existing reads.
#[derive(Debug, Clone, Default)]
pub struct EvidenceSources {
    pub artifacts: Vec<ThreadArtifact>,
    pub result: Option<ThreadResult>,
    pub land_gate: Vec<LandGateVerdict>,
    pub packets: Vec<ReviewPacket>,
    pub verdicts: Vec<ReviewVerdict>,
}

impl EvidenceTimeline {
    pub fn assemble(thread_id: ThreadId, thread_version: i64, sources: EvidenceSources) -> Self {
        let result_sha = sources
            .result
            .as_ref()
            .and_then(|r| result_sha256(&r.result).ok());
        let mut current_artifacts: Vec<String> =
            sources.artifacts.iter().map(|a| a.sha256.clone()).collect();
        current_artifacts.sort();
        let current = EvidenceManifest {
            thread_id,
            result: match (&sources.result, &result_sha) {
                (Some(r), Some(sha)) => Some(ResultEvidence {
                    sha256: sha.clone(),
                    produced_by: r.produced_by,
                }),
                _ => None,
            },
            artifacts: current_artifacts,
            attestations: Vec::new(),
        };
        let evidence_changed_since_review = sources
            .packets
            .iter()
            .max_by_key(|p| (p.created_at, p.id))
            .map(|latest| !latest.manifest.same_evidence(&current));

        let mut entries: Vec<EvidenceTimelineEntry> = Vec::new();
        entries.extend(sources.artifacts.into_iter().map(|artifact| {
            EvidenceTimelineEntry::ArtifactLinked {
                at: artifact.linked_at,
                artifact,
            }
        }));
        entries.extend(
            sources
                .result
                .map(|r| EvidenceTimelineEntry::ResultRecorded {
                    at: r.produced_at,
                    sha256: result_sha,
                    produced_by: r.produced_by,
                }),
        );
        entries.extend(sources.land_gate.into_iter().map(|verdict| {
            EvidenceTimelineEntry::LandGateVerdict {
                at: verdict.recorded_at,
                verdict,
            }
        }));
        entries.extend(sources.packets.into_iter().map(|packet| {
            EvidenceTimelineEntry::ReviewRequested {
                at: packet.created_at,
                packet,
            }
        }));
        entries.extend(sources.verdicts.into_iter().map(|verdict| {
            EvidenceTimelineEntry::ReviewVerdict {
                at: verdict.recorded_at,
                verdict,
            }
        }));
        // Stable, so entries of one kind keep the order their read gave.
        entries.sort_by_key(|e| (e.at(), e.rank()));
        Self {
            thread_id,
            thread_version,
            evidence_changed_since_review,
            entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::ReviewDecision;
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_790_000_000 + secs, 0)
            .single()
            .unwrap_or_default()
    }

    fn member() -> MemberId {
        MemberId(uuid::Uuid::new_v4())
    }

    fn artifact(thread: ThreadId, sha: &str, at: i64) -> ThreadArtifact {
        ThreadArtifact {
            thread_id: thread,
            sha256: sha.into(),
            linked_by: member(),
            linked_at: t(at),
        }
    }

    fn packet(
        thread: ThreadId,
        artifacts: &[&str],
        result: Option<ResultEvidence>,
        at: i64,
    ) -> ReviewPacket {
        ReviewPacket {
            id: uuid::Uuid::new_v4(),
            thread_id: thread,
            requested_by: member(),
            thread_version: at,
            manifest: EvidenceManifest {
                thread_id: thread,
                result,
                artifacts: artifacts.iter().map(|s| s.to_string()).collect(),
                attestations: Vec::new(),
            },
            evidence_root: format!("root-{at}"),
            self_reported_only: false,
            created_at: t(at),
        }
    }

    fn verdict(thread: ThreadId, at: i64) -> ReviewVerdict {
        ReviewVerdict {
            id: at,
            thread_id: thread,
            reviewer_id: member(),
            decision: ReviewDecision::Approve,
            note: None,
            actor_id: None,
            head_sha: None,
            recorded_at: t(at),
        }
    }

    fn kinds(timeline: &EvidenceTimeline) -> Vec<String> {
        timeline
            .entries
            .iter()
            .map(|e| {
                serde_json::to_value(e)
                    .ok()
                    .and_then(|v| v["kind"].as_str().map(str::to_string))
                    .unwrap_or_default()
            })
            .collect()
    }

    #[test]
    fn entries_read_oldest_first_and_a_hand_off_precedes_its_verdicts() {
        let thread = ThreadId(uuid::Uuid::new_v4());
        let timeline = EvidenceTimeline::assemble(
            thread,
            7,
            EvidenceSources {
                artifacts: vec![artifact(thread, "bb", 10), artifact(thread, "aa", 30)],
                packets: vec![packet(thread, &["aa", "bb"], None, 30)],
                verdicts: vec![verdict(thread, 30), verdict(thread, 40)],
                ..Default::default()
            },
        );
        assert_eq!(
            kinds(&timeline),
            [
                "artifact_linked",
                "artifact_linked",
                "review_requested",
                "review_verdict",
                "review_verdict"
            ]
        );
        assert_eq!(timeline.thread_version, 7);
        assert_eq!(timeline.evidence_changed_since_review, Some(false));
    }

    #[test]
    fn evidence_that_moved_after_the_latest_hand_off_is_flagged() {
        let thread = ThreadId(uuid::Uuid::new_v4());
        let sources = |artifacts: Vec<ThreadArtifact>| EvidenceSources {
            artifacts,
            packets: vec![
                packet(thread, &[], None, 5),
                packet(thread, &["aa"], None, 20),
            ],
            ..Default::default()
        };
        let same = EvidenceTimeline::assemble(thread, 1, sources(vec![artifact(thread, "aa", 10)]));
        assert_eq!(
            same.evidence_changed_since_review,
            Some(false),
            "judged against the latest packet"
        );
        let moved = EvidenceTimeline::assemble(
            thread,
            1,
            sources(vec![artifact(thread, "aa", 10), artifact(thread, "cc", 25)]),
        );
        assert_eq!(moved.evidence_changed_since_review, Some(true));
        let never = EvidenceTimeline::assemble(thread, 0, EvidenceSources::default());
        assert_eq!(
            never.evidence_changed_since_review, None,
            "never handed to review"
        );
        assert!(never.entries.is_empty());
    }

    #[test]
    fn a_result_is_pinned_by_the_same_hash_a_packet_uses() {
        let thread = ThreadId(uuid::Uuid::new_v4());
        let producer = member();
        let payload = serde_json::json!({ "b": 1, "a": 2 });
        let sha = result_sha256(&payload).unwrap_or_default();
        let timeline = EvidenceTimeline::assemble(
            thread,
            3,
            EvidenceSources {
                result: Some(ThreadResult {
                    thread_id: thread,
                    result: payload,
                    produced_by: producer,
                    produced_at: t(1),
                }),
                packets: vec![packet(
                    thread,
                    &[],
                    Some(ResultEvidence {
                        sha256: sha.clone(),
                        produced_by: producer,
                    }),
                    2,
                )],
                ..Default::default()
            },
        );
        assert_eq!(timeline.evidence_changed_since_review, Some(false));
        assert!(matches!(
            &timeline.entries[0],
            EvidenceTimelineEntry::ResultRecorded { sha256: Some(s), .. } if *s == sha
        ));
    }
}
