//! The human-timeout state machine: an agent waiting on a person does not
//! hold its thread forever.
//!
//! An agent that declares `needs_input` or `needs_review` has stopped until a
//! person answers, and before this it kept its claim (and, through its
//! heartbeat, its lease) however long the person took: the work sat with an
//! agent that could not move it, and nobody else could pick it up. Now the
//! wait runs on [`HumanWaitState`]'s table. It is `waiting` from the
//! declaration, `overdue` from half of `MAIDAN_HUMAN_TIMEOUT_SECS` (3600 s by
//! default; `0` turns it off), and at the full timeout the claim reaper's
//! tick returns the thread to the queue: one store transaction releases the
//! holder's claim, drops the stale declaration and appends `WaitTimedOut`
//! with policy [`RETURN_TO_QUEUE`], which the notification router takes to
//! the owner. A person's answer clears the declaration, which is `answered`,
//! and the clock is gone.
//!
//! Return to the queue is the only ending, by decision: the thread stays
//! open and nothing is approved, closed or proceeded with on the person's
//! behalf. Only the holder's own declaration on a live, acknowledged lease
//! starts the clock; an owner's declaration, a lease-less assignment and a
//! lapsed lease (the reaper's own business) do not.

use std::time::Duration;

use chrono::{DateTime, Utc};
use maidan_types::{
    DeclaredStatus, HumanWait, HumanWaitInput, HumanWaitState, Thread, ThreadState,
};
use metrics::counter;

use crate::state::AppState;

/// Default seconds an agent may wait on a person before its thread goes
/// back to the queue.
pub const DEFAULT_HUMAN_TIMEOUT_SECS: u64 = 3600;

/// The `WaitTimedOut` policy a returned wait carries.
pub const RETURN_TO_QUEUE: &str = "return_to_queue";

/// Threads returned per store call, and the most per tick, so a backlog
/// cannot hold a reaper tick open.
const BATCH: i64 = 100;
const MAX_PER_TICK: usize = 1000;

/// Read `MAIDAN_HUMAN_TIMEOUT_SECS`. An unset or unparsable value keeps the
/// default and `0` turns the machine off (`None`).
pub fn timeout_from_env() -> Option<Duration> {
    timeout_from_raw(std::env::var("MAIDAN_HUMAN_TIMEOUT_SECS").ok())
}

fn timeout_from_raw(raw: Option<String>) -> Option<Duration> {
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_HUMAN_TIMEOUT_SECS);
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// Statuses that mean the holder stopped for a person. `blocked` is left
/// out because what blocks it may be another thread, not a person.
pub fn waits_on_a_person(status: DeclaredStatus) -> bool {
    matches!(
        status,
        DeclaredStatus::NeedsInput | DeclaredStatus::NeedsReview
    )
}

/// Where `thread`'s wait on a person stands at `now`, by walking the
/// machine through what has elapsed. Reads [`Thread::status`], so the caller
/// attaches the active declaration first.
pub fn judge(thread: &Thread, now: DateTime<Utc>, timeout: Option<Duration>) -> Option<HumanWait> {
    let timeout = chrono::Duration::from_std(timeout?).ok()?;
    let declared = thread.status.as_ref()?;
    let holder = thread.assignee_id?;
    let lease = thread.assignment_expires_at?;
    let held = thread.state == ThreadState::Open
        && thread.work_started_at.is_some()
        && lease >= now
        && declared.declared_by == holder;
    if !held || !waits_on_a_person(declared.status) {
        return None;
    }
    let since = declared.declared_at;
    // An out-of-range timeout means the deadline never comes.
    let returns_at = since.checked_add_signed(timeout)?;
    let warn_at = since.checked_add_signed(timeout / 2)?;
    let mut state = HumanWaitState::Waiting;
    for (at, input) in [
        (warn_at, HumanWaitInput::WarnElapsed),
        (returns_at, HumanWaitInput::TimeoutElapsed),
    ] {
        if at <= now {
            // The inputs arrive in table order, so a refusal cannot happen;
            // keeping the last good state is the safe reading if it did.
            state = state.next(input).unwrap_or(state);
        }
    }
    Some(HumanWait {
        state,
        since,
        returns_at,
    })
}

/// Set each thread's [`Thread::human_wait`] at the current time and the
/// configured timeout.
pub fn mark<'a>(threads: impl IntoIterator<Item = &'a mut Thread>) {
    let timeout = timeout_from_env();
    let now = Utc::now();
    for thread in threads {
        thread.human_wait = judge(thread, now, timeout);
    }
}

/// Return to the queue every thread whose holder has waited on a person
/// longer than `timeout`, up to [`MAX_PER_TICK`], and publish each
/// `WaitTimedOut`. Runs on the claim reaper's tick. Returns how many went
/// back.
pub async fn return_timed_out_once(state: &AppState, timeout: Duration) -> usize {
    let now = Utc::now();
    let Some(declared_before) = chrono::Duration::from_std(timeout)
        .ok()
        .and_then(|timeout| now.checked_sub_signed(timeout))
    else {
        return 0;
    };
    let mut returned = 0;
    while returned < MAX_PER_TICK {
        let events = match state
            .store
            .return_human_timeouts(now, declared_before, RETURN_TO_QUEUE, BATCH)
            .await
        {
            Ok(events) => events,
            Err(err) => {
                tracing::warn!(error = %err, "human_wait.return_failed");
                break;
            }
        };
        let batch = events.len();
        for stored in events {
            crate::routes::publish_stored(state, stored).await;
        }
        returned += batch;
        counter!("maidan_human_waits_returned_total").increment(batch as u64);
        if batch < BATCH as usize {
            break;
        }
    }
    returned
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::{ChannelId, MemberId, ThreadId, ThreadStatusDeclaration};

    const HOUR: Option<Duration> = Some(Duration::from_secs(3600));

    fn waiting(mins_ago: i64, status: DeclaredStatus, now: DateTime<Utc>) -> Thread {
        let holder = MemberId(uuid::Uuid::new_v4());
        let id = ThreadId(uuid::Uuid::new_v4());
        let since = now - chrono::Duration::minutes(mins_ago);
        Thread {
            id,
            channel_id: ChannelId(uuid::Uuid::new_v4()),
            parent_thread_id: None,
            title: None,
            description: None,
            state: ThreadState::Open,
            assignee_id: Some(holder),
            assignment_expires_at: Some(now + chrono::Duration::minutes(5)),
            claim_lease_id: None,
            work_started_at: Some(since - chrono::Duration::minutes(1)),
            owner_id: None,
            created_by: None,
            attributes: None,
            created_at: since,
            updated_at: now,
            tombstoned_at: None,
            status: Some(ThreadStatusDeclaration {
                thread_id: id,
                status,
                note: "which schema?".into(),
                declared_by: holder,
                declared_at: since,
            }),
            block: None,
            closed_without_review: false,
            notices: Vec::new(),
            last_progress_at: None,
            stalled: false,
            human_wait: None,
            plan_state: None,
            ci: Vec::new(),
        }
    }

    fn state_at(mins_ago: i64) -> Option<HumanWaitState> {
        let now = Utc::now();
        judge(
            &waiting(mins_ago, DeclaredStatus::NeedsInput, now),
            now,
            HOUR,
        )
        .map(|w| w.state)
    }

    #[test]
    fn a_wait_on_a_person_is_waiting_then_overdue_then_returned() {
        assert_eq!(state_at(10), Some(HumanWaitState::Waiting));
        assert_eq!(state_at(30), Some(HumanWaitState::Overdue));
        assert_eq!(state_at(45), Some(HumanWaitState::Overdue));
        assert_eq!(state_at(60), Some(HumanWaitState::ReturnedToQueue));
    }

    #[test]
    fn the_deadline_is_the_declaration_plus_the_timeout() {
        let now = Utc::now();
        let thread = waiting(10, DeclaredStatus::NeedsReview, now);
        let wait = judge(&thread, now, HOUR);
        let since = now - chrono::Duration::minutes(10);
        assert_eq!(
            wait,
            Some(HumanWait {
                state: HumanWaitState::Waiting,
                since,
                returns_at: since + chrono::Duration::hours(1),
            })
        );
    }

    #[test]
    fn only_a_holders_wait_on_a_person_on_a_live_acknowledged_lease_counts() {
        let now = Utc::now();
        for other in [
            DeclaredStatus::Working,
            DeclaredStatus::Blocked,
            DeclaredStatus::Done,
        ] {
            assert_eq!(
                judge(&waiting(10, other, now), now, HOUR),
                None,
                "{other:?}"
            );
        }
        let mut by_owner = waiting(10, DeclaredStatus::NeedsInput, now);
        if let Some(declared) = by_owner.status.as_mut() {
            declared.declared_by = MemberId(uuid::Uuid::new_v4());
        }
        assert_eq!(
            judge(&by_owner, now, HOUR),
            None,
            "someone else's declaration"
        );
        let mut lapsed = waiting(10, DeclaredStatus::NeedsInput, now);
        lapsed.assignment_expires_at = Some(now - chrono::Duration::seconds(1));
        assert_eq!(
            judge(&lapsed, now, HOUR),
            None,
            "a lapsed lease is the reaper's"
        );
        let mut unacknowledged = waiting(10, DeclaredStatus::NeedsInput, now);
        unacknowledged.work_started_at = None;
        assert_eq!(judge(&unacknowledged, now, HOUR), None);
        let mut closed = waiting(10, DeclaredStatus::NeedsInput, now);
        closed.state = ThreadState::Closed;
        assert_eq!(judge(&closed, now, HOUR), None);
        let mut answered = waiting(10, DeclaredStatus::NeedsInput, now);
        answered.status = None;
        assert_eq!(
            judge(&answered, now, HOUR),
            None,
            "an answer clears the clock"
        );
    }

    #[test]
    fn off_or_out_of_range_timeouts_never_judge_and_do_not_panic() {
        let now = Utc::now();
        let thread = waiting(10, DeclaredStatus::NeedsInput, now);
        assert_eq!(judge(&thread, now, None), None);
        let huge = Some(Duration::from_secs(1_000_000_000_000_000));
        assert_eq!(judge(&thread, now, huge), None);
    }

    #[test]
    fn the_timeout_defaults_to_an_hour_and_zero_turns_it_off() {
        assert_eq!(timeout_from_raw(None), HOUR);
        assert_eq!(timeout_from_raw(Some("soon".into())), HOUR);
        assert_eq!(
            timeout_from_raw(Some(" 120 ".into())),
            Some(Duration::from_secs(120))
        );
        assert_eq!(timeout_from_raw(Some("0".into())), None);
    }
}
