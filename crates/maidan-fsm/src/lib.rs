//! FSM engine for Maidan agent threads.
//!
//! Pure transition logic: maps `(ThreadState, ThreadAction)` to the next
//! state or [`FsmError::InvalidTransition`]. Persistence and HTTP wiring
//! live in `maidan-store` and `maidan-server` (Cluster D.3).

use maidan_types::ThreadState;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadAction {
    StartReview,
    Close,
    Archive,
}

impl ThreadAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StartReview => "start_review",
            Self::Close => "close",
            Self::Archive => "archive",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "start_review" => Some(Self::StartReview),
            "close" => Some(Self::Close),
            "archive" => Some(Self::Archive),
            _ => None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("invalid transition from {from:?} via {action:?}")]
pub struct InvalidTransition {
    pub from: ThreadState,
    pub action: ThreadAction,
}

/// Apply `action` to a thread in `from`. Returns the next state on success.
pub fn apply(from: ThreadState, action: ThreadAction) -> Result<ThreadState, InvalidTransition> {
    let next = match (from, action) {
        (ThreadState::Open, ThreadAction::StartReview) => ThreadState::InReview,
        (ThreadState::InReview, ThreadAction::Close) => ThreadState::Closed,
        (ThreadState::Closed, ThreadAction::Archive) => ThreadState::Archived,
        (from, action) => return Err(InvalidTransition { from, action }),
    };
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legal_open_to_in_review() {
        assert_eq!(
            apply(ThreadState::Open, ThreadAction::StartReview).unwrap(),
            ThreadState::InReview
        );
    }

    #[test]
    fn legal_in_review_to_closed() {
        assert_eq!(
            apply(ThreadState::InReview, ThreadAction::Close).unwrap(),
            ThreadState::Closed
        );
    }

    #[test]
    fn legal_closed_to_archived() {
        assert_eq!(
            apply(ThreadState::Closed, ThreadAction::Archive).unwrap(),
            ThreadState::Archived
        );
    }

    #[test]
    fn illegal_from_archived() {
        for action in [
            ThreadAction::StartReview,
            ThreadAction::Close,
            ThreadAction::Archive,
        ] {
            assert_eq!(
                apply(ThreadState::Archived, action),
                Err(InvalidTransition {
                    from: ThreadState::Archived,
                    action,
                })
            );
        }
    }

    #[test]
    fn illegal_skip_in_review() {
        assert_eq!(
            apply(ThreadState::Open, ThreadAction::Close),
            Err(InvalidTransition {
                from: ThreadState::Open,
                action: ThreadAction::Close,
            })
        );
    }

    #[test]
    fn action_parse_roundtrip() {
        for action in [
            ThreadAction::StartReview,
            ThreadAction::Close,
            ThreadAction::Archive,
        ] {
            assert_eq!(ThreadAction::parse(action.as_str()), Some(action));
        }
        assert!(ThreadAction::parse("reopen").is_none());
    }
}
