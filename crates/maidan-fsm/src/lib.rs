//! FSM engine for Maidan agent threads.
//!
//! Pure transition logic: maps `(ThreadState, ThreadAction)` to the next
//! state or [`InvalidTransition`]. Persistence and HTTP wiring live in
//! `maidan-store` and `maidan-server` (Cluster D.3).

pub mod hsm;

use maidan_types::ThreadState;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadAction {
    StartReview,
    Close,
    Archive,
    /// A reviewer sends the work back for another round. Reached only through a
    /// `request_changes` review, which carries the reviewer's note, so it is not
    /// one of the actions [`ThreadAction::parse`] accepts.
    RequestChanges,
}

impl ThreadAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StartReview => "start_review",
            Self::Close => "close",
            Self::Archive => "archive",
            Self::RequestChanges => "request_changes",
        }
    }

    /// The actions a caller may name directly on a transition route.
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
        (ThreadState::InReview, ThreadAction::RequestChanges) => ThreadState::Open,
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
    fn a_change_request_returns_review_to_open() {
        assert_eq!(
            apply(ThreadState::InReview, ThreadAction::RequestChanges).unwrap(),
            ThreadState::Open
        );
        assert!(apply(ThreadState::Open, ThreadAction::RequestChanges).is_err());
        assert!(apply(ThreadState::Closed, ThreadAction::RequestChanges).is_err());
    }

    #[test]
    fn request_changes_is_not_a_route_action() {
        assert_eq!(ThreadAction::parse("request_changes"), None);
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
