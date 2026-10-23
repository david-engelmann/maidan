//! A workspace's own retention: how long it keeps its messages, events,
//! finished deliveries and unreferenced media, and how much media it may hold,
//! within what the instance keeps.
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

/// Days each kind of row is kept. `None` means that side did not set a
/// cutoff. A message ceiling comes from `MAIDAN_RETENTION_MESSAGES_DAYS`;
/// when that is unset, messages stay until a workspace policy prunes them.
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
    /// Days a media artifact (attachment, screenshot, recording) is kept once
    /// nothing in the workspace references it. The instance's comes from
    /// `MAIDAN_RETENTION_MEDIA_DAYS` (30 unless set).
    #[serde(default)]
    pub media_days: Option<i64>,
    /// Bytes of media artifacts the workspace may hold. An upload that would
    /// go past it is refused with a `media-quota-exceeded` 409. The
    /// instance's comes from `MAIDAN_MEDIA_QUOTA_BYTES` (20 GiB unless set).
    #[serde(default)]
    pub media_quota_bytes: Option<i64>,
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
            ("media_days", self.media_days, instance.media_days),
        ] {
            let Some(days) = days else { continue };
            if !(1..=MAX_RETENTION_DAYS).contains(&days) {
                return Err(format!(
                    "{kind} must be between 1 and {MAX_RETENTION_DAYS}, got {days}"
                ));
            }
            if let Some(ceiling) = ceiling
                && days > ceiling
            {
                return Err(format!(
                    "{kind} is {days}, longer than this instance keeps them ({ceiling}); \
                         a workspace may keep rows for less time than the instance, not more"
                ));
            }
        }
        if let Some(bytes) = self.media_quota_bytes {
            if bytes < 1 {
                return Err(format!("media_quota_bytes must be at least 1, got {bytes}"));
            }
            if let Some(ceiling) = instance.media_quota_bytes
                && bytes > ceiling
            {
                return Err(format!(
                    "media_quota_bytes is {bytes}, more than this instance allows ({ceiling}); \
                         a workspace may hold less media than the instance allows, not more"
                ));
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
            media_days: min(self.media_days, other.media_days),
            media_quota_bytes: min(self.media_quota_bytes, other.media_quota_bytes),
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
            ..RetentionDays::default()
        }
    }

    fn media(days: Option<i64>, quota: Option<i64>) -> RetentionDays {
        RetentionDays {
            media_days: days,
            media_quota_bytes: quota,
            ..RetentionDays::default()
        }
    }

    #[test]
    fn a_workspace_may_keep_less_media_than_the_instance_never_more() {
        let instance = media(Some(30), Some(1_000));
        assert!(media(Some(30), Some(1_000)).check_within(&instance).is_ok());
        assert!(media(Some(1), Some(1)).check_within(&instance).is_ok());
        let err = media(Some(31), None).check_within(&instance).unwrap_err();
        assert!(err.contains("media_days") && err.contains("30"), "{err}");
        let err = media(None, Some(1_001))
            .check_within(&instance)
            .unwrap_err();
        assert!(
            err.contains("media_quota_bytes") && err.contains("1000"),
            "{err}"
        );
        for bad in [0, -1] {
            assert!(media(None, Some(bad))
                .check_within(&RetentionDays::default())
                .is_err());
            assert!(media(Some(bad), None)
                .check_within(&RetentionDays::default())
                .is_err());
        }
        // With expiry and the quota off on the instance, any value in range
        // is a workspace's own choice.
        assert!(media(Some(3650), Some(i64::MAX))
            .check_within(&RetentionDays::default())
            .is_ok());
    }

    #[test]
    fn the_effective_media_settings_are_the_stricter_of_the_two() {
        let instance = media(Some(30), Some(1_000));
        assert_eq!(
            media(Some(7), None).stricter(&instance),
            media(Some(7), Some(1_000))
        );
        assert_eq!(
            media(None, Some(10)).stricter(&RetentionDays::default()),
            media(None, Some(10))
        );
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
    fn a_message_ceiling_refuses_a_longer_workspace_policy() {
        let instance = days(Some(30), None, None);
        assert!(days(Some(30), None, None).check_within(&instance).is_ok());
        let err = days(Some(31), None, None)
            .check_within(&instance)
            .unwrap_err();
        assert!(err.contains("messages_days") && err.contains("30"), "{err}");
        // No instance message ceiling: 3650 is still inside the day range.
        assert!(days(Some(3650), None, None)
            .check_within(&RetentionDays::default())
            .is_ok());
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
        let parsed: RetentionDays =
            serde_json::from_str(r#"{"media_days": 3, "media_quota_bytes": 4}"#).unwrap();
        assert_eq!(parsed, media(Some(3), Some(4)));
    }
}
