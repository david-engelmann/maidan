//! Stuck detection: a thread whose agent went quiet is marked `stalled` on
//! the board's reads.
//!
//! The claim reaper catches an agent that died, because its lease lapses, but
//! not one that hung: a heartbeat that still renews keeps the lease alive
//! while the work goes nowhere, and the supervisor sees a card that looks
//! busy. The store reports when the holder last showed progress on the thread
//! ([`Thread::last_progress_at`]); a thread is stalled once that is
//! `MAIDAN_STALL_SECS` old (900 s by default; `0` turns it off) and the
//! holder still holds a live lease. A holder that declared it is waiting
//! (`needs_input`, `needs_review`, `blocked`, `done`) is not stalled: it said
//! why it is quiet, and the board shows that declaration instead.
//!
//! It is judged at read time, so no loop and no stored flag go stale; a
//! notice to the owner when a thread stalls would need a record of who was
//! told, and is not part of this.

use std::time::Duration;

use chrono::{DateTime, Utc};
use maidan_types::{DeclaredStatus, Thread};

/// Default seconds without progress before a worked thread is stalled.
pub const DEFAULT_STALL_SECS: u64 = 900;

/// Read `MAIDAN_STALL_SECS`. An unset or unparsable value keeps the 900 s
/// default and `0` turns detection off (`None`).
pub fn threshold_from_env() -> Option<Duration> {
    threshold_from_raw(std::env::var("MAIDAN_STALL_SECS").ok())
}

fn threshold_from_raw(raw: Option<String>) -> Option<Duration> {
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_STALL_SECS);
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// Whether `thread` is stalled at `now`. Reads [`Thread::status`], so the
/// caller attaches the active declaration first.
pub fn is_stalled(thread: &Thread, now: DateTime<Utc>, threshold: Option<Duration>) -> bool {
    let (Some(threshold), Some(progress), Some(lease)) = (
        threshold,
        thread.last_progress_at,
        thread.assignment_expires_at,
    ) else {
        return false;
    };
    // A lapsed lease is the reaper's to report and return to the queue; the
    // card is not "stuck" with an agent any more.
    if lease < now {
        return false;
    }
    if thread
        .status
        .as_ref()
        .is_some_and(|declared| declared.status != DeclaredStatus::Working)
    {
        return false;
    }
    let Ok(threshold) = chrono::Duration::from_std(threshold) else {
        return false;
    };
    // An operator may set an enormous threshold to all but turn detection
    // off; past chrono's date range the deadline never comes, so not stalled
    // rather than a panic on every board read.
    progress
        .checked_add_signed(threshold)
        .is_some_and(|deadline| deadline <= now)
}

/// Mark each thread's [`Thread::stalled`] at the current time and the
/// configured threshold.
pub fn mark<'a>(threads: impl IntoIterator<Item = &'a mut Thread>) {
    let threshold = threshold_from_env();
    let now = Utc::now();
    for thread in threads {
        thread.stalled = is_stalled(thread, now, threshold);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::{ChannelId, MemberId, ThreadId, ThreadState, ThreadStatusDeclaration};

    fn worked(progress_mins_ago: i64, now: DateTime<Utc>) -> Thread {
        let started = now - chrono::Duration::minutes(progress_mins_ago);
        Thread {
            id: ThreadId(uuid::Uuid::new_v4()),
            channel_id: ChannelId(uuid::Uuid::new_v4()),
            parent_thread_id: None,
            title: None,
            description: None,
            state: ThreadState::Open,
            assignee_id: Some(MemberId(uuid::Uuid::new_v4())),
            assignment_expires_at: Some(now + chrono::Duration::minutes(5)),
            claim_lease_id: None,
            work_started_at: Some(started),
            owner_id: None,
            created_by: None,
            created_at: started,
            updated_at: now,
            tombstoned_at: None,
            status: None,
            block: None,
            closed_without_review: false,
            notices: Vec::new(),
            last_progress_at: Some(started),
            stalled: false,
            human_wait: None,
        }
    }

    const FIFTEEN_MINS: Option<Duration> = Some(Duration::from_secs(900));

    #[test]
    fn a_holder_quiet_past_the_threshold_on_a_live_lease_is_stalled() {
        let now = Utc::now();
        assert!(is_stalled(&worked(20, now), now, FIFTEEN_MINS));
        assert!(is_stalled(&worked(15, now), now, FIFTEEN_MINS));
        assert!(!is_stalled(&worked(14, now), now, FIFTEEN_MINS));
    }

    #[test]
    fn a_lapsed_lease_is_the_reapers_not_a_stall() {
        let now = Utc::now();
        let mut thread = worked(20, now);
        thread.assignment_expires_at = Some(now - chrono::Duration::seconds(1));
        assert!(!is_stalled(&thread, now, FIFTEEN_MINS));
    }

    #[test]
    fn a_declared_wait_is_not_a_stall_but_a_declared_working_is() {
        let now = Utc::now();
        let mut thread = worked(20, now);
        let declare = |status| ThreadStatusDeclaration {
            thread_id: thread.id,
            status,
            note: "n".into(),
            declared_by: MemberId(uuid::Uuid::new_v4()),
            declared_at: now - chrono::Duration::minutes(20),
        };
        for waiting in [
            DeclaredStatus::NeedsInput,
            DeclaredStatus::NeedsReview,
            DeclaredStatus::Blocked,
            DeclaredStatus::Done,
        ] {
            let declared = declare(waiting);
            thread.status = Some(declared);
            assert!(!is_stalled(&thread, now, FIFTEEN_MINS), "{waiting:?}");
        }
        thread.status = Some(declare(DeclaredStatus::Working));
        assert!(is_stalled(&thread, now, FIFTEEN_MINS));
    }

    #[test]
    fn no_progress_clock_or_detection_off_is_never_stalled() {
        let now = Utc::now();
        let mut thread = worked(60, now);
        assert!(!is_stalled(&thread, now, None));
        thread.last_progress_at = None;
        assert!(!is_stalled(&thread, now, FIFTEEN_MINS));
    }

    #[test]
    fn a_threshold_past_the_date_range_is_never_stalled_and_does_not_panic() {
        let now = Utc::now();
        let huge = Some(Duration::from_secs(1_000_000_000_000_000));
        assert!(!is_stalled(&worked(60, now), now, huge));
    }

    #[test]
    fn the_threshold_defaults_to_fifteen_minutes_and_zero_turns_it_off() {
        assert_eq!(threshold_from_raw(None), FIFTEEN_MINS);
        assert_eq!(threshold_from_raw(Some("nope".into())), FIFTEEN_MINS);
        assert_eq!(
            threshold_from_raw(Some(" 60 ".into())),
            Some(Duration::from_secs(60))
        );
        assert_eq!(threshold_from_raw(Some("0".into())), None);
    }
}
