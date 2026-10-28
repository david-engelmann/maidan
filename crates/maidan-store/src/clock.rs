//! Timestamps written into columns the database keeps at microsecond resolution.
//!
//! Postgres `timestamptz` stores microseconds and rounds a nanosecond reading
//! on the way in. SQLite stores the RFC 3339 text we bind, nanoseconds and
//! all. A claim writes `claimed_at` and `updated_at` as one instant, and the
//! unacknowledged-claim event names `claimed_at` so a caller can match it to
//! the claim row's `updated_at`. A reading that still carries nanoseconds does
//! not survive that round trip: the row comes back at microseconds, and the
//! event payload (JSON) can disagree with the row by the digits Postgres
//! dropped, or by a microsecond when the driver and the database round
//! opposite ways. Two separate `Utc::now()` calls straddle that boundary
//! often enough to fail the comparison and rarely enough to look like a flake.
//! Truncating once, before either column is bound, makes both backends store
//! the same instant the event reports.

use chrono::{DateTime, Duration, Utc};

/// `Utc::now()`, truncated to whole microseconds.
pub(crate) fn db_now() -> DateTime<Utc> {
    truncate_micros(Utc::now())
}

/// Drop the nanoseconds below one microsecond. Floor, not round: the value is
/// already a valid microsecond instant, so neither backend adjusts it again.
pub(crate) fn truncate_micros(instant: DateTime<Utc>) -> DateTime<Utc> {
    let stray = i64::from(instant.timestamp_subsec_nanos() % 1_000);
    instant - Duration::nanoseconds(stray)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};

    use super::truncate_micros;

    #[test]
    fn a_timestamp_keeps_only_whole_microseconds() {
        let instant = Utc
            .timestamp_opt(1_700_000_000, 349_013_789)
            .single()
            .expect("in range");
        let truncated = truncate_micros(instant);
        assert_eq!(truncated.timestamp_subsec_nanos(), 349_013_000);
        assert_eq!(truncated.timestamp(), instant.timestamp());
        assert_eq!(truncate_micros(truncated), truncated);

        let aligned = Utc
            .timestamp_opt(1_700_000_000, 349_013_000)
            .single()
            .expect("in range");
        assert_eq!(truncate_micros(aligned), aligned);
    }

    #[test]
    fn a_microsecond_timestamp_matches_itself_after_the_event_payload_round_trip() {
        let instant = truncate_micros(
            Utc.timestamp_opt(1_700_000_000, 349_013_789)
                .single()
                .expect("in range"),
        );
        let payload = serde_json::to_value(instant).expect("serialize");
        let back: DateTime<Utc> = serde_json::from_value(payload).expect("deserialize");
        assert_eq!(back, instant);
    }
}
