//! Attestation tiers: how far each piece of evidence handed to review can be
//! trusted, judged once in the hand-off's transaction and pinned in the packet
//! (Open Work Next 3). Both backends gather the facts; the rules live here
//! once.

use std::collections::HashSet;

use maidan_types::{
    is_qualifying_pass, AttestationTier, EvidenceAttestation, EvidenceKind, EvidenceManifest,
    LandColor, LandGateStatus, MemberId,
};

/// Who put a piece of evidence in front of the reviewers: the member, and the
/// delegate that acted with the member's token when there was one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Author {
    pub member: MemberId,
    pub actor: Option<MemberId>,
}

impl Author {
    /// A worker's evidence is the work's own account, whoever's token carried
    /// it: a delegate that worked the thread does not launder its evidence
    /// through a member who did not.
    fn is_worker(&self, workers: &HashSet<MemberId>) -> bool {
        workers.contains(&self.member) || self.actor.is_some_and(|a| workers.contains(&a))
    }

    fn tier(&self, workers: &HashSet<MemberId>) -> AttestationTier {
        if self.is_worker(workers) {
            AttestationTier::SelfReported
        } else {
            AttestationTier::Attached
        }
    }
}

/// A land-gate verdict as the close gate reads it.
#[derive(Debug, Clone)]
pub(crate) struct RecordedPass {
    pub status: LandGateStatus,
    pub land: LandColor,
    pub recorded_by: MemberId,
    pub artifact_sha: Option<String>,
    pub owner_id: Option<MemberId>,
    pub assignee_id: Option<MemberId>,
    /// The recorder, or the delegate that recorded with its token, owned,
    /// holds or once held the thread.
    pub worked: bool,
    pub skilled: bool,
}

impl RecordedPass {
    /// The close gate's own test, so a pass that would not let the thread
    /// close never verifies its evidence either.
    pub fn qualifies(&self) -> bool {
        is_qualifying_pass(
            self.status,
            self.land,
            self.recorded_by,
            self.owner_id,
            self.assignee_id,
            self.worked,
            self.skilled,
        )
    }
}

/// The thread's land-gate row as the close gate reads it.
#[derive(Debug, Clone)]
pub(crate) enum GateReading {
    /// No row: the gate is not armed.
    Unarmed,
    /// Armed, with no verdict recorded yet.
    Pending,
    Recorded(RecordedPass),
}

/// The tier of every piece of evidence in `manifest`, in a fixed order (the
/// result, the artifacts as the manifest sorts them, then the land-gate pass)
/// so the same facts always give the same root.
///
/// - `self_reported`: the result or an artifact put there by one of the
///   thread's workers (or a delegate that worked it).
/// - `attached`: the result or an artifact put there by anyone else.
/// - `verified`: a land-gate pass standing at the hand-off that the close gate
///   would accept.
pub(crate) fn attest(
    manifest: &EvidenceManifest,
    result_author: Option<Author>,
    link_authors: &[(String, Author)],
    workers: &HashSet<MemberId>,
    gate: &GateReading,
) -> Vec<EvidenceAttestation> {
    let mut out = Vec::new();
    if let (Some(result), Some(author)) = (&manifest.result, result_author) {
        out.push(EvidenceAttestation {
            kind: EvidenceKind::Result,
            sha256: Some(result.sha256.clone()),
            tier: author.tier(workers),
            attested_by: author.member,
        });
    }
    for sha in &manifest.artifacts {
        if let Some((_, author)) = link_authors.iter().find(|(s, _)| s == sha) {
            out.push(EvidenceAttestation {
                kind: EvidenceKind::Artifact,
                sha256: Some(sha.clone()),
                tier: author.tier(workers),
                attested_by: author.member,
            });
        }
    }
    if let GateReading::Recorded(pass) = gate {
        if pass.qualifies() {
            out.push(EvidenceAttestation {
                kind: EvidenceKind::LandGate,
                sha256: pass.artifact_sha.clone(),
                tier: AttestationTier::Verified,
                attested_by: pass.recorded_by,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::{ResultEvidence, ThreadId};

    fn member(n: u128) -> MemberId {
        MemberId(uuid::Uuid::from_u128(n))
    }

    fn manifest(result_by: Option<MemberId>, artifacts: &[&str]) -> EvidenceManifest {
        EvidenceManifest {
            thread_id: ThreadId(uuid::Uuid::nil()),
            result: result_by.map(|by| ResultEvidence {
                sha256: "r".repeat(64),
                produced_by: by,
            }),
            artifacts: artifacts.iter().map(|s| s.to_string()).collect(),
            attestations: Vec::new(),
        }
    }

    fn by(m: MemberId) -> Author {
        Author {
            member: m,
            actor: None,
        }
    }

    fn pass(recorded_by: MemberId, worked: bool, skilled: bool, land: LandColor) -> GateReading {
        GateReading::Recorded(RecordedPass {
            status: LandGateStatus::Pass,
            land,
            recorded_by,
            artifact_sha: Some("a".repeat(64)),
            owner_id: Some(member(9)),
            assignee_id: None,
            worked,
            skilled,
        })
    }

    #[test]
    fn a_workers_evidence_is_self_reported_and_anyone_elses_is_attached() {
        let (worker, owner) = (member(1), member(2));
        let workers = HashSet::from([worker]);
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        let tiers = attest(
            &manifest(Some(worker), &[&a, &b]),
            Some(by(worker)),
            &[(a.clone(), by(worker)), (b.clone(), by(owner))],
            &workers,
            &GateReading::Unarmed,
        );
        let got: Vec<_> = tiers.iter().map(|t| (t.kind, t.tier)).collect();
        assert_eq!(
            got,
            vec![
                (EvidenceKind::Result, AttestationTier::SelfReported),
                (EvidenceKind::Artifact, AttestationTier::SelfReported),
                (EvidenceKind::Artifact, AttestationTier::Attached),
            ]
        );
        assert!(!maidan_types::self_reported_only(&tiers));
        assert!(maidan_types::self_reported_only(&tiers[..2]));
    }

    #[test]
    fn a_delegate_that_worked_the_thread_does_not_launder_its_evidence() {
        let (worker, reviewer) = (member(1), member(3));
        let workers = HashSet::from([worker]);
        let borrowed = Author {
            member: reviewer,
            actor: Some(worker),
        };
        let a = "a".repeat(64);
        let tiers = attest(
            &manifest(Some(reviewer), &[&a]),
            Some(borrowed),
            &[(a.clone(), borrowed)],
            &workers,
            &GateReading::Unarmed,
        );
        assert!(tiers
            .iter()
            .all(|t| t.tier == AttestationTier::SelfReported));
        assert!(tiers.iter().all(|t| t.attested_by == reviewer));
    }

    #[test]
    fn only_a_pass_the_close_gate_would_accept_is_verified() {
        let (worker, verifier) = (member(1), member(4));
        let workers = HashSet::from([worker]);
        let m = manifest(Some(worker), &[]);
        let verified = |gate: &GateReading| {
            attest(&m, Some(by(worker)), &[], &workers, gate)
                .iter()
                .any(|t| t.tier == AttestationTier::Verified)
        };
        assert!(verified(&pass(verifier, false, true, LandColor::Green)));
        assert!(!verified(&pass(verifier, false, true, LandColor::Amber)));
        assert!(!verified(&pass(verifier, false, false, LandColor::Green)));
        assert!(!verified(&pass(verifier, true, true, LandColor::Green)));
        assert!(!verified(&pass(member(9), false, true, LandColor::Green)));
        assert!(!verified(&GateReading::Pending));
        assert!(!verified(&GateReading::Unarmed));
    }

    #[test]
    fn no_evidence_never_warns() {
        let tiers = attest(
            &manifest(None, &[]),
            None,
            &[],
            &HashSet::new(),
            &GateReading::Unarmed,
        );
        assert!(tiers.is_empty());
        assert!(!maidan_types::self_reported_only(&tiers));
    }
}
