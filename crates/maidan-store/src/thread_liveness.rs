//! Refusing a write aimed at a thread that has ended or is gone.
//!
//! A write that lands on a tombstoned or ended thread and answers success
//! tells the caller its work counted when nothing will ever read it. Each
//! write names the states it refuses: a message may still be posted to a
//! closed thread, because people discuss after a close, but a footprint or
//! a required skill only steers dispatch, which an ended thread no longer
//! takes part in.

use maidan_types::{Thread, ThreadId, ThreadState};

use crate::StoreError;

/// Refuse a write to a tombstoned thread with [`StoreError::Gone`].
pub fn refuse_gone(thread: &Thread) -> Result<(), StoreError> {
    refuse_gone_at(thread.id, thread.tombstoned_at.is_some())
}

fn refuse_gone_at(thread_id: ThreadId, gone: bool) -> Result<(), StoreError> {
    if gone {
        return Err(StoreError::Gone(format!(
            "thread {} was deleted; nothing was written",
            thread_id.0
        )));
    }
    Ok(())
}

/// Refuse a message to a thread that is gone or archived. A closed thread
/// takes messages.
pub fn refuse_post(thread: &Thread) -> Result<(), StoreError> {
    refuse_gone(thread)?;
    if thread.state == ThreadState::Archived {
        return Err(StoreError::Ended(format!(
            "thread {} is archived and takes no new messages; reopen it first",
            thread.id.0
        )));
    }
    Ok(())
}

/// Refuse `what` on a thread that is gone, closed or archived.
pub fn refuse_ended(thread: &Thread, what: &str) -> Result<(), StoreError> {
    refuse_ended_at(
        thread.id,
        thread.state,
        thread.tombstoned_at.is_some(),
        what,
    )
}

/// [`refuse_ended`] on the columns a locked row read: its `state` and
/// whether it is tombstoned.
pub(crate) fn refuse_ended_at(
    thread_id: ThreadId,
    state: ThreadState,
    gone: bool,
    what: &str,
) -> Result<(), StoreError> {
    refuse_gone_at(thread_id, gone)?;
    if state.is_terminal() {
        return Err(StoreError::Ended(format!(
            "thread {} is {}; {what} applies only to a thread that is open or in review, \
             and nothing was written",
            thread_id.0,
            state.as_str()
        )));
    }
    Ok(())
}

/// [`refuse_ended_at`] on a `state` column read as text.
pub(crate) fn refuse_ended_column(
    thread_id: ThreadId,
    state: &str,
    gone: bool,
    what: &str,
) -> Result<(), StoreError> {
    let state = ThreadState::parse(state)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown thread state `{state}`")))?;
    refuse_ended_at(thread_id, state, gone, what)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use maidan_types::{ChannelId, ThreadId};

    use super::*;

    fn thread(state: ThreadState, tombstoned: bool) -> Thread {
        let mut thread: Thread = serde_json::from_value(serde_json::json!({
            "id": ThreadId::new().0,
            "channel_id": ChannelId::new().0,
            "parent_thread_id": null,
            "title": null,
            "state": "open",
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "tombstoned_at": null,
        }))
        .expect("a minimal thread");
        thread.state = state;
        thread.tombstoned_at = tombstoned.then(Utc::now);
        thread
    }

    #[test]
    fn a_message_is_refused_on_a_gone_or_archived_thread_and_taken_on_a_closed_one() {
        assert!(matches!(
            refuse_post(&thread(ThreadState::Open, true)),
            Err(StoreError::Gone(_))
        ));
        assert!(matches!(
            refuse_post(&thread(ThreadState::Archived, false)),
            Err(StoreError::Ended(_))
        ));
        assert!(refuse_post(&thread(ThreadState::Closed, false)).is_ok());
        assert!(refuse_post(&thread(ThreadState::InReview, false)).is_ok());
    }

    #[test]
    fn a_dispatch_write_is_refused_on_a_closed_or_archived_thread() {
        for state in [ThreadState::Closed, ThreadState::Archived] {
            assert!(matches!(
                refuse_ended(&thread(state, false), "a footprint"),
                Err(StoreError::Ended(_))
            ));
        }
        for state in [ThreadState::Open, ThreadState::InReview] {
            assert!(refuse_ended(&thread(state, false), "a footprint").is_ok());
        }
        assert!(matches!(
            refuse_ended(&thread(ThreadState::Closed, true), "a footprint"),
            Err(StoreError::Gone(_))
        ));
    }
}
