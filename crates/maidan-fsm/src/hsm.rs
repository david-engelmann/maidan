//! Hierarchical rules: a child thread's state cannot outrun its parent.

use maidan_types::ThreadState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HsmError {
    ParentArchived,
    ChildAheadOfParent,
}

impl HsmError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParentArchived => "parent thread is archived",
            Self::ChildAheadOfParent => "child state cannot advance beyond parent state",
        }
    }
}

/// Ordinal used for parent/child ordering (higher = further along lifecycle).
pub fn state_rank(state: ThreadState) -> u8 {
    match state {
        ThreadState::Open => 0,
        ThreadState::InReview => 1,
        ThreadState::Closed => 2,
        ThreadState::Archived => 3,
    }
}

/// When `parent` is set, ensure `child_to` is allowed relative to the parent's state.
pub fn parent_allows_transition(
    parent: ThreadState,
    child_to: ThreadState,
) -> Result<(), HsmError> {
    if parent == ThreadState::Archived {
        return Err(HsmError::ParentArchived);
    }
    if state_rank(child_to) > state_rank(parent) {
        return Err(HsmError::ChildAheadOfParent);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_may_match_parent_progress() {
        assert!(parent_allows_transition(ThreadState::InReview, ThreadState::InReview).is_ok());
        assert!(parent_allows_transition(ThreadState::Closed, ThreadState::Open).is_ok());
    }

    #[test]
    fn child_cannot_outrun_parent() {
        assert_eq!(
            parent_allows_transition(ThreadState::Open, ThreadState::InReview),
            Err(HsmError::ChildAheadOfParent)
        );
    }

    #[test]
    fn archived_parent_blocks_child() {
        assert_eq!(
            parent_allows_transition(ThreadState::Archived, ThreadState::Open),
            Err(HsmError::ParentArchived)
        );
    }
}
