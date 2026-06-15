//! Property tests for the thread FSM and the hierarchical (parent/child)
//! rank rule. The unit tests in `src/` pin the individual legal edges; these
//! prove the *invariants* hold for arbitrary states, action sequences, and
//! arbitrary parent/child trees.

use maidan_fsm::hsm::{parent_allows_transition, state_rank, HsmError};
use maidan_fsm::{apply, ThreadAction};
use maidan_types::ThreadState;
use proptest::prelude::*;

fn any_state() -> impl Strategy<Value = ThreadState> {
    prop_oneof![
        Just(ThreadState::Open),
        Just(ThreadState::InReview),
        Just(ThreadState::Closed),
        Just(ThreadState::Archived),
    ]
}

fn any_action() -> impl Strategy<Value = ThreadAction> {
    prop_oneof![
        Just(ThreadAction::StartReview),
        Just(ThreadAction::Close),
        Just(ThreadAction::Archive),
    ]
}

/// Independent specification of the legal edges — deliberately *not* sharing
/// code with `apply`, so the table and the implementation cross-check.
fn spec_next(from: ThreadState, action: ThreadAction) -> Option<ThreadState> {
    match (from, action) {
        (ThreadState::Open, ThreadAction::StartReview) => Some(ThreadState::InReview),
        (ThreadState::InReview, ThreadAction::Close) => Some(ThreadState::Closed),
        (ThreadState::Closed, ThreadAction::Archive) => Some(ThreadState::Archived),
        _ => None,
    }
}

proptest! {
    /// `apply` succeeds exactly on the three legal edges, and the success/error
    /// payloads match the specification for every `(state, action)` pair.
    #[test]
    fn apply_matches_the_edge_specification(from in any_state(), action in any_action()) {
        match (apply(from, action), spec_next(from, action)) {
            (Ok(next), Some(expected)) => prop_assert_eq!(next, expected),
            (Err(err), None) => {
                prop_assert_eq!(err.from, from);
                prop_assert_eq!(err.action, action);
            }
            (got, expected) => {
                prop_assert!(false, "apply={got:?} but spec={expected:?} for ({from:?}, {action:?})")
            }
        }
    }

    /// Every legal transition advances the lifecycle rank by exactly one and
    /// never originates from the terminal state. This is a structural invariant
    /// that does not restate the edge table.
    #[test]
    fn a_legal_transition_advances_rank_by_exactly_one(from in any_state(), action in any_action()) {
        if let Ok(next) = apply(from, action) {
            prop_assert_ne!(from, ThreadState::Archived);
            prop_assert_eq!(i16::from(state_rank(next)), i16::from(state_rank(from)) + 1);
        }
    }

    /// `Archived` is terminal: no action transitions out of it.
    #[test]
    fn archived_is_terminal(action in any_action()) {
        prop_assert!(apply(ThreadState::Archived, action).is_err());
    }

    /// Driving a thread with an arbitrary sequence of actions never regresses
    /// its rank, never exceeds the maximum rank, and never escapes `Archived`.
    /// (An illegal action leaves the caller holding the prior state.)
    #[test]
    fn rank_is_monotonic_under_arbitrary_action_sequences(
        start in any_state(),
        actions in prop::collection::vec(any_action(), 0..24),
    ) {
        let mut state = start;
        for action in actions {
            let prev_rank = state_rank(state);
            let was_archived = state == ThreadState::Archived;
            if let Ok(next) = apply(state, action) {
                prop_assert!(!was_archived, "a transition fired from Archived");
                prop_assert!(state_rank(next) >= prev_rank);
                state = next;
            }
            prop_assert!(state_rank(state) >= prev_rank, "rank regressed");
            prop_assert!(state_rank(state) <= state_rank(ThreadState::Archived));
        }
    }

    /// The HSM rule: a transition is allowed iff the parent is not archived and
    /// the child's target rank does not exceed the parent's rank.
    #[test]
    fn parent_allows_transition_respects_the_rank_ceiling(
        parent in any_state(),
        child_to in any_state(),
    ) {
        let result = parent_allows_transition(parent, child_to);
        if parent == ThreadState::Archived {
            prop_assert_eq!(result, Err(HsmError::ParentArchived));
        } else if state_rank(child_to) > state_rank(parent) {
            prop_assert_eq!(result, Err(HsmError::ChildAheadOfParent));
        } else {
            prop_assert!(result.is_ok());
        }
    }

    /// For an arbitrary rooted tree of threads: if every direct parent→child
    /// edge is individually permitted by the engine, then the invariant holds
    /// globally — no descendant outruns any ancestor, and no internal node is
    /// archived. This is the "arbitrary trees" exit criterion: the local check
    /// composes into a sound tree-wide guarantee.
    #[test]
    fn locally_valid_tree_is_globally_consistent(
        states in prop::collection::vec(any_state(), 1..12),
        // parent_seeds[i-1] picks node i's parent as (seed % i) < i — a forest
        // edge that is always acyclic with node 0 as the root.
        parent_seeds in prop::collection::vec(any::<usize>(), 11),
    ) {
        let n = states.len();
        let parent_of = |i: usize| -> usize { parent_seeds[i - 1] % i };

        let all_edges_ok =
            (1..n).all(|i| parent_allows_transition(states[parent_of(i)], states[i]).is_ok());

        if all_edges_ok {
            // No descendant outranks any of its ancestors.
            for start in 0..n {
                let mut cur = start;
                while cur != 0 {
                    let p = parent_of(cur);
                    prop_assert!(state_rank(states[cur]) <= state_rank(states[p]));
                    cur = p;
                }
            }
            // No internal (has-child) node is archived — an archived parent
            // could never have produced an allowed edge.
            let mut has_child = vec![false; n];
            for i in 1..n {
                has_child[parent_of(i)] = true;
            }
            for (i, &has) in has_child.iter().enumerate() {
                if has {
                    prop_assert_ne!(states[i], ThreadState::Archived);
                }
            }
        }
    }

    /// `ThreadAction::parse` accepts exactly the three known wire strings and
    /// rejects all other snake-case noise; `parse ∘ as_str` round-trips.
    #[test]
    fn parse_accepts_exactly_the_known_actions(s in "[a-z_]{0,16}") {
        let known = ["start_review", "close", "archive"];
        prop_assert_eq!(ThreadAction::parse(&s).is_some(), known.contains(&s.as_str()));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `parse ∘ as_str` is the identity on every action variant.
    #[test]
    fn as_str_then_parse_round_trips(action in any_action()) {
        prop_assert_eq!(ThreadAction::parse(action.as_str()), Some(action));
    }
}
