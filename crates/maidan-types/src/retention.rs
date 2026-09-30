//! A workspace's own retention: how long it keeps its messages, events and
//! finished deliveries, within what the instance keeps.
//!
//! The instance's `MAIDAN_RETENTION_*_DAYS` are the ceiling. A workspace may
//! keep a kind of row for less time than the instance does, never for more:
//! the instance sweep prunes past its own cutoff whatever a workspace says. A
//! legal hold outranks both, so a held workspace's rows are kept whatever its
//! policy says. Audit rows have no workspace setting: a tenant does not
//! shorten the record of what was done in it.

use serde::{Deserialize, Serialize};

use crate::WorkspaceId;

/// The longest retention a workspace may set, in days (ten years).
pub const MAX_RETENTION_DAYS: i64 = 3650;

/// Days each kind of row is kept. `None` keeps it as long as the instance
/// does (for messages, which the instance never prunes: forever).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct RetentionDays {
    #[serde(default)]
    pub messages_days: Option<i64>,
    #[serde(default)]
    pub events_days: Option<i64>,
    #[serde(default)]
    pub deliveries_days: Option<i64>,
}

impl RetentionDays {
    /// Nothing set: the workspace keeps what the instance keeps.
    pub fn is_unset(&self) -> bool {
        *self == Self::default()
    }

    /// Refuse a value outside `1..=MAX_RETENTION_DAYS`, or one longer than the
    /// instance keeps that kind of row. The error names the kind and the bound.
    pub fn check_within(&self, instance: &RetentionDays) -> Result<(), String> {
        for (kind, days, ceiling) in [
            ("messages_days", self.messages_days, instance.messages_days),
            ("events_days", self.events_days, instance.events_days),
            (
                "deliveries_days",
                self.deliveries_days,
                instance.deliveries_days,
            ),
        ] {
            let Some(days) = days else { continue };
            if !(1..=MAX_RETENTION_DAYS).contains(&days) {
                return Err(format!(
                    "{kind} must be between 1 and {MAX_RETENTION_DAYS}, got {days}"
                ));
            }
            if let Some(ceiling) = ceiling {
                if days > ceiling {
                    return Err(format!(
                        "{kind} is {days}, longer than this instance keeps them ({ceiling}); \
                         a workspace may keep rows for less time than the instance, not more"
                    ));
                }
            }
        }
        Ok(())
    }

    /// Per kind, the shorter of the two; `None` only where neither is set.
    pub fn stricter(&self, other: &RetentionDays) -> RetentionDays {
        fn min(a: Option<i64>, b: Option<i64>) -> Option<i64> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        }
        RetentionDays {
            messages_days: min(self.messages_days, other.messages_days),
            events_days: min(self.events_days, other.events_days),
            deliveries_days: min(self.deliveries_days, other.deliveries_days),
        }
    }
}

/// A workspace's retention as `GET /workspaces/{id}/retention` reports it: what
/// the workspace set, what the instance keeps, and what is pruned in effect
/// (the shorter of the two per kind).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RetentionPolicy {
    pub workspace_id: WorkspaceId,
    pub workspace: RetentionDays,
    pub instance: RetentionDays,
    pub effective: RetentionDays,
}

impl RetentionPolicy {
    pub fn new(
        workspace_id: WorkspaceId,
        workspace: RetentionDays,
        instance: RetentionDays,
    ) -> Self {
        Self {
            workspace_id,
            workspace,
            instance,
            effective: workspace.stricter(&instance),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(m: Option<i64>, e: Option<i64>, d: Option<i64>) -> RetentionDays {
        RetentionDays {
            messages_days: m,
            events_days: e,
            deliveries_days: d,
        }
    }

    #[test]
    fn a_workspace_may_be_stricter_than_the_instance_never_longer() {
        let instance = days(None, Some(30), Some(14));
        assert!(days(Some(1), Some(1), Some(14))
            .check_within(&instance)
            .is_ok());
        assert!(days(Some(3650), None, None).check_within(&instance).is_ok());
        let err = days(None, Some(31), None)
            .check_within(&instance)
            .unwrap_err();
        assert!(err.contains("events_days") && err.contains("30"), "{err}");
        assert!(days(None, None, Some(15)).check_within(&instance).is_err());
    }

    #[test]
    fn days_outside_the_range_are_refused_whatever_the_instance() {
        let open = RetentionDays::default();
        for bad in [0, -1, MAX_RETENTION_DAYS + 1] {
            assert!(days(Some(bad), None, None).check_within(&open).is_err());
        }
    }

    #[test]
    fn the_effective_retention_is_the_shorter_per_kind() {
        let workspace = days(Some(7), Some(60), None);
        let instance = days(None, Some(30), Some(14));
        assert_eq!(
            RetentionPolicy::new(WorkspaceId::new(), workspace, instance).effective,
            days(Some(7), Some(30), Some(14))
        );
    }

    #[test]
    fn an_unknown_field_is_refused() {
        assert!(serde_json::from_str::<RetentionDays>(r#"{"audit_days": 1}"#).is_err());
        let parsed: RetentionDays = serde_json::from_str(r#"{"events_days": 2}"#).unwrap();
        assert_eq!(parsed, days(None, Some(2), None));
    }
}
