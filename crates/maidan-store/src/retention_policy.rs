//! Per-workspace retention, backend-neutral: the instance ceiling a workspace's
//! policy is checked against, read from the same variables the sweeper uses.

use maidan_types::RetentionDays;

use crate::StoreError;

/// Days an unreferenced media artifact is kept when
/// `MAIDAN_RETENTION_MEDIA_DAYS` is unset.
pub const DEFAULT_MEDIA_DAYS: u32 = 30;

/// Bytes of media a workspace may hold when `MAIDAN_MEDIA_QUOTA_BYTES` is
/// unset: 20 GiB.
pub const DEFAULT_MEDIA_QUOTA_BYTES: i64 = 20 * 1024 * 1024 * 1024;

/// A retention knob in days: unset, `0` or junk mean "not pruned".
pub fn parse_days(raw: Option<&str>) -> Option<u32> {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&d| d > 0)
}

/// `MAIDAN_RETENTION_MEDIA_DAYS`. Unlike the other day knobs it is on by
/// default, so unset (or blank) cannot mean "off": it means
/// [`DEFAULT_MEDIA_DAYS`]. Any value set is read as the others are read, so
/// `0`, `off` and anything that is not a positive whole number keep media
/// forever. A typo errs toward keeping data, as it does for the others.
pub fn parse_media_days(raw: Option<&str>) -> Option<u32> {
    match raw.map(str::trim) {
        None | Some("") => Some(DEFAULT_MEDIA_DAYS),
        Some(set) => {
            let days = parse_days(Some(set));
            // A typo keeps media, the safe direction, but it turns off an
            // expiry that is on by default, so the operator is told.
            if days.is_none() && set != "0" && !set.eq_ignore_ascii_case("off") {
                tracing::warn!(
                    value = set,
                    "MAIDAN_RETENTION_MEDIA_DAYS is not a whole number of days, 0 or off; media is not expired"
                );
            }
            days
        }
    }
}

/// `MAIDAN_MEDIA_QUOTA_BYTES`. Unset (or blank) is
/// [`DEFAULT_MEDIA_QUOTA_BYTES`]; `0` or `off` turns the quota off. A value
/// that is neither a positive whole number nor one of those keeps the
/// default: a typo must not lift the cap.
pub fn parse_media_quota_bytes(raw: Option<&str>) -> Option<i64> {
    let Some(set) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Some(DEFAULT_MEDIA_QUOTA_BYTES);
    };
    if set == "0" || set.eq_ignore_ascii_case("off") {
        return None;
    }
    match set.parse::<i64>() {
        Ok(bytes) if bytes > 0 => Some(bytes),
        _ => {
            tracing::warn!(
                value = set,
                "MAIDAN_MEDIA_QUOTA_BYTES is not a positive number of bytes; keeping the default"
            );
            Some(DEFAULT_MEDIA_QUOTA_BYTES)
        }
    }
}

/// What the instance keeps, from `MAIDAN_RETENTION_MESSAGES_DAYS`,
/// `MAIDAN_RETENTION_EVENTS_DAYS`, `MAIDAN_RETENTION_DELIVERIES_DAYS`,
/// `MAIDAN_RETENTION_MEDIA_DAYS` and `MAIDAN_MEDIA_QUOTA_BYTES`. Unset means
/// the instance does not prune that kind of row, except for media, which is
/// bounded by default.
pub fn instance_retention_from_env() -> RetentionDays {
    let var = |name: &str| std::env::var(name).ok();
    let days = |name: &str| parse_days(var(name).as_deref()).map(i64::from);
    RetentionDays {
        messages_days: days("MAIDAN_RETENTION_MESSAGES_DAYS"),
        events_days: days("MAIDAN_RETENTION_EVENTS_DAYS"),
        deliveries_days: days("MAIDAN_RETENTION_DELIVERIES_DAYS"),
        media_days: parse_media_days(var("MAIDAN_RETENTION_MEDIA_DAYS").as_deref()).map(i64::from),
        media_quota_bytes: parse_media_quota_bytes(var("MAIDAN_MEDIA_QUOTA_BYTES").as_deref()),
    }
}

/// Refuse a policy outside the day range or longer than the instance keeps.
pub(crate) fn validate(days: &RetentionDays, instance: &RetentionDays) -> Result<(), StoreError> {
    days.check_within(instance)
        .map_err(StoreError::InvalidInput)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_days_default_to_thirty_and_zero_or_off_keeps_media() {
        assert_eq!(parse_media_days(None), Some(30));
        assert_eq!(parse_media_days(Some("  ")), Some(30));
        assert_eq!(parse_media_days(Some("7")), Some(7));
        assert_eq!(parse_media_days(Some("0")), None);
        assert_eq!(parse_media_days(Some("off")), None);
    }

    #[test]
    fn the_media_quota_defaults_to_twenty_gib_and_zero_or_off_lifts_it() {
        assert_eq!(parse_media_quota_bytes(None), Some(21_474_836_480));
        assert_eq!(parse_media_quota_bytes(Some("")), Some(21_474_836_480));
        assert_eq!(parse_media_quota_bytes(Some("1024")), Some(1024));
        assert_eq!(parse_media_quota_bytes(Some("0")), None);
        assert_eq!(parse_media_quota_bytes(Some("OFF")), None);
        assert_eq!(
            parse_media_quota_bytes(Some("20GiB")),
            Some(21_474_836_480),
            "a typo keeps the cap"
        );
    }
}
