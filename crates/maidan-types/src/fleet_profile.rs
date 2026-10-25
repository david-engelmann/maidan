//! Fleet harness profiles: what every member of a fleet runs so the fleet
//! shares one cached system and tools prefix, and the conformance check that
//! reads the usage ledger's reporter evidence against it.
//!
//! A profile names the harness build (harness and version), the provider, the
//! model, and the boot pack whose sha256 every call should carry. An unset
//! field is not checked. The check is pure: it reads completed ledger entries
//! and reports, per member, how many conform and which values diverge.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{MemberId, UsageLedgerEntry, WorkspaceId};

/// The published build a fleet runs. Every field is optional; an unset field
/// is not checked.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct FleetProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// sha256, hex, of the boot pack every call must list in `pack_sha256`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_pack_sha256: Option<String>,
}

/// The profile field a report diverged on.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FleetField {
    Harness,
    HarnessVersion,
    Provider,
    Model,
    BootPack,
}

/// One divergent value and how many of the member's reports carried it.
/// `seen` is absent when the report sent no value (or, for the boot pack, did
/// not list it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FleetDivergence {
    pub field: FleetField,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<String>,
    pub reports: i64,
}

/// One member's reports against the profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberConformance {
    pub member_id: MemberId,
    pub reports: i64,
    pub conforming: i64,
    /// Sorted by field, then value.
    pub divergences: Vec<FleetDivergence>,
}

impl MemberConformance {
    pub fn conforms(&self) -> bool {
        self.conforming == self.reports
    }
}

/// A workspace's recent completed reports against a fleet profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FleetConformance {
    pub workspace_id: WorkspaceId,
    pub profile: FleetProfile,
    pub reports: i64,
    pub conforming: i64,
    /// Whether every report read conforms. True when there are none.
    pub conforms: bool,
    /// Sorted by member id.
    pub members: Vec<MemberConformance>,
}

fn norm(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

impl FleetProfile {
    /// The profile with blank fields dropped and the rest trimmed, so `?harness=`
    /// checks nothing rather than demanding an empty value.
    pub fn normalized(&self) -> Self {
        let n = |v: &Option<String>| norm(v.as_deref()).map(str::to_owned);
        Self {
            harness: n(&self.harness),
            harness_version: n(&self.harness_version),
            provider: n(&self.provider),
            model: n(&self.model),
            boot_pack_sha256: n(&self.boot_pack_sha256).map(|s| s.to_ascii_lowercase()),
        }
    }

    /// Where one report departs from the profile, in field order. Empty when it
    /// conforms. Expects a normalized profile.
    pub fn divergences(&self, entry: &UsageLedgerEntry) -> Vec<(FleetField, Option<String>)> {
        let evidence = &entry.evidence;
        let mut out = Vec::new();
        let mut check = |field, want: &Option<String>, seen: Option<&str>| {
            if let Some(want) = want {
                let seen = norm(seen);
                if seen != Some(want.as_str()) {
                    out.push((field, seen.map(str::to_owned)));
                }
            }
        };
        check(
            FleetField::Harness,
            &self.harness,
            evidence.harness.as_deref(),
        );
        check(
            FleetField::HarnessVersion,
            &self.harness_version,
            evidence.harness_version.as_deref(),
        );
        check(
            FleetField::Provider,
            &self.provider,
            evidence.provider.as_deref(),
        );
        check(
            FleetField::Model,
            &self.model,
            Some(entry.stamp.model.as_str()),
        );
        if let Some(pack) = &self.boot_pack_sha256 {
            let listed = evidence
                .pack_sha256
                .iter()
                .any(|p| p.trim().eq_ignore_ascii_case(pack));
            if !listed {
                out.push((FleetField::BootPack, None));
            }
        }
        out
    }

    /// Check a workspace's completed reports against the profile.
    pub fn check(
        &self,
        workspace_id: WorkspaceId,
        entries: &[UsageLedgerEntry],
    ) -> FleetConformance {
        let profile = self.normalized();
        type Tally = (i64, i64, BTreeMap<(FleetField, Option<String>), i64>);
        let mut members: BTreeMap<uuid::Uuid, Tally> = BTreeMap::new();
        for entry in entries {
            let tally = members.entry(entry.stamp.reporter.0).or_default();
            tally.0 += 1;
            let divergent = profile.divergences(entry);
            if divergent.is_empty() {
                tally.1 += 1;
            }
            for key in divergent {
                *tally.2.entry(key).or_default() += 1;
            }
        }
        let members: Vec<MemberConformance> = members
            .into_iter()
            .map(|(id, (reports, conforming, divergent))| MemberConformance {
                member_id: MemberId(id),
                reports,
                conforming,
                divergences: divergent
                    .into_iter()
                    .map(|((field, seen), reports)| FleetDivergence {
                        field,
                        seen,
                        reports,
                    })
                    .collect(),
            })
            .collect();
        let reports = members.iter().map(|m| m.reports).sum();
        let conforming = members.iter().map(|m| m.conforming).sum();
        FleetConformance {
            workspace_id,
            profile,
            reports,
            conforming,
            conforms: conforming == reports,
            members,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ClaimLeaseId, PayerStamp, PriceSnapshot, ThreadBudget, ThreadId, TokenUsage, UsageEvidence,
    };

    fn entry(reporter: MemberId, model: &str, evidence: UsageEvidence) -> UsageLedgerEntry {
        UsageLedgerEntry {
            usage_report_id: uuid::Uuid::new_v4(),
            thread_id: ThreadId(uuid::Uuid::new_v4()),
            stamp: PayerStamp {
                payer: WorkspaceId(uuid::Uuid::nil()),
                reporter,
                claim_lease_id: ClaimLeaseId(uuid::Uuid::new_v4()),
                model: model.into(),
                tokens: TokenUsage::default(),
                usd_micros: 0,
                price_snapshot: PriceSnapshot::default(),
            },
            turns: 1,
            evidence,
            budget: serde_json::from_value::<ThreadBudget>(serde_json::json!({
                "thread_id": uuid::Uuid::nil(),
                "used_tokens": 0,
                "used_usd_micros": 0,
                "used_turns": 0,
                "created_at": "2026-10-25T00:00:00Z",
                "updated_at": "2026-10-25T00:00:00Z"
            }))
            .unwrap(),
            stopped: false,
            reason: None,
            usage_event_id: 1,
            claim_failed_event_id: None,
            accepted_at: chrono::Utc::now(),
        }
    }

    fn built(harness: &str, version: &str, packs: &[&str]) -> UsageEvidence {
        UsageEvidence {
            provider: Some("anthropic".into()),
            harness: Some(harness.into()),
            harness_version: Some(version.into()),
            pack_sha256: packs.iter().map(|p| (*p).to_owned()).collect(),
            ..Default::default()
        }
    }

    fn profile() -> FleetProfile {
        FleetProfile {
            harness: Some("claude-code".into()),
            harness_version: Some("2.1.0".into()),
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet".into()),
            boot_pack_sha256: Some("ABC123".into()),
        }
    }

    #[test]
    fn a_report_on_the_published_build_conforms() {
        let a = MemberId(uuid::Uuid::new_v4());
        let report = profile().check(
            WorkspaceId(uuid::Uuid::nil()),
            &[entry(
                a,
                "claude-sonnet",
                built("claude-code", "2.1.0", &["abc123"]),
            )],
        );
        assert!(report.conforms);
        assert_eq!((report.reports, report.conforming), (1, 1));
        assert!(report.members[0].divergences.is_empty());
    }

    #[test]
    fn a_member_on_another_version_or_without_the_boot_pack_is_named_with_the_value_it_sent() {
        let a = MemberId(uuid::Uuid::new_v4());
        let entries = [
            entry(
                a,
                "claude-sonnet",
                built("claude-code", "2.0.9", &["abc123"]),
            ),
            entry(a, "claude-sonnet", built("claude-code", "2.0.9", &[])),
            entry(
                a,
                "claude-sonnet",
                built("claude-code", "2.1.0", &["abc123"]),
            ),
        ];
        let report = profile().check(WorkspaceId(uuid::Uuid::nil()), &entries);
        assert!(!report.conforms);
        let member = &report.members[0];
        assert_eq!((member.reports, member.conforming), (3, 1));
        assert_eq!(
            member.divergences,
            vec![
                FleetDivergence {
                    field: FleetField::HarnessVersion,
                    seen: Some("2.0.9".into()),
                    reports: 2
                },
                FleetDivergence {
                    field: FleetField::BootPack,
                    seen: None,
                    reports: 1
                },
            ]
        );
    }

    #[test]
    fn a_report_that_sent_no_harness_diverges_with_no_seen_value() {
        let a = MemberId(uuid::Uuid::new_v4());
        let report = profile().check(
            WorkspaceId(uuid::Uuid::nil()),
            &[entry(a, "claude-sonnet", UsageEvidence::default())],
        );
        let fields: Vec<_> = report.members[0]
            .divergences
            .iter()
            .map(|d| (d.field, d.seen.clone()))
            .collect();
        assert_eq!(
            fields,
            vec![
                (FleetField::Harness, None),
                (FleetField::HarnessVersion, None),
                (FleetField::Provider, None),
                (FleetField::BootPack, None),
            ]
        );
    }

    #[test]
    fn an_unset_or_blank_profile_field_is_not_checked() {
        let a = MemberId(uuid::Uuid::new_v4());
        let only_harness = FleetProfile {
            harness: Some(" claude-code ".into()),
            harness_version: Some("  ".into()),
            ..Default::default()
        };
        let report = only_harness.check(
            WorkspaceId(uuid::Uuid::nil()),
            &[entry(a, "any/model", built("claude-code", "9.9.9", &[]))],
        );
        assert!(report.conforms);
        assert_eq!(report.profile.harness_version, None);
        assert_eq!(report.profile.harness.as_deref(), Some("claude-code"));
    }

    #[test]
    fn members_are_reported_separately_and_an_empty_ledger_conforms() {
        let a = MemberId(uuid::Uuid::from_u128(1));
        let b = MemberId(uuid::Uuid::from_u128(2));
        let entries = [
            entry(b, "claude-sonnet", built("other", "2.1.0", &["abc123"])),
            entry(
                a,
                "claude-sonnet",
                built("claude-code", "2.1.0", &["abc123"]),
            ),
        ];
        let report = profile().check(WorkspaceId(uuid::Uuid::nil()), &entries);
        assert_eq!(report.members.len(), 2);
        assert_eq!(report.members[0].member_id, a);
        assert!(report.members[0].conforms());
        assert!(!report.members[1].conforms());
        assert_eq!((report.reports, report.conforming), (2, 1));

        let empty = profile().check(WorkspaceId(uuid::Uuid::nil()), &[]);
        assert!(empty.conforms);
        assert!(empty.members.is_empty());
    }
}
