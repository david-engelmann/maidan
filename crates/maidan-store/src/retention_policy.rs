//! Per-workspace retention, backend-neutral: the instance ceiling a workspace's
//! policy is checked against, read from the same variables the sweeper uses.

use maidan_types::RetentionDays;

use crate::StoreError;

/// A retention knob in days: unset, `0` or junk mean "not pruned".
pub fn parse_days(raw: Option<&str>) -> Option<u32> {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&d| d > 0)
}

/// What the instance keeps, from `MAIDAN_RETENTION_EVENTS_DAYS` and
/// `MAIDAN_RETENTION_DELIVERIES_DAYS`. The instance never prunes messages, so
/// their ceiling is always `None`.
pub fn instance_retention_from_env() -> RetentionDays {
    let days = |var: &str| parse_days(std::env::var(var).ok().as_deref()).map(i64::from);
    RetentionDays {
        messages_days: None,
        events_days: days("MAIDAN_RETENTION_EVENTS_DAYS"),
        deliveries_days: days("MAIDAN_RETENTION_DELIVERIES_DAYS"),
    }
}

/// Refuse a policy outside the day range or longer than the instance keeps.
pub(crate) fn validate(days: &RetentionDays, instance: &RetentionDays) -> Result<(), StoreError> {
    days.check_within(instance)
        .map_err(StoreError::InvalidInput)
}
