# Cluster 112.0 retro — FSM property tests

> Tag **`v112.0.0`**. Second cluster of Phase XXI (correctness & coverage).

## What shipped

- **`maidan-fsm` property-test suite** — the crate had per-edge unit tests
  but no invariant coverage. Added `tests/fsm_properties.rs` (8 `proptest`
  properties) proving the engine's guarantees hold for arbitrary states,
  action sequences, and arbitrary parent/child trees. (112.0.1, #310)
  - **`apply_matches_the_edge_specification`** — for every `(state, action)`,
    `apply` succeeds exactly on the three legal edges and the success/error
    payload matches an *independent* edge table (`spec_next`), so the table
    and the implementation cross-check rather than share a `match`.
  - **`a_legal_transition_advances_rank_by_exactly_one`** — a structural
    invariant (rank +1, never from `Archived`) that doesn't restate the edge
    table.
  - **`archived_is_terminal`** — no action transitions out of `Archived`.
  - **`rank_is_monotonic_under_arbitrary_action_sequences`** — driving with a
    random action sequence never regresses rank, never exceeds the max, never
    escapes `Archived`.
  - **`parent_allows_transition_respects_the_rank_ceiling`** — the HSM rule
    for every `(parent, child_to)`: allowed iff parent not archived and child
    target rank ≤ parent rank.
  - **`locally_valid_tree_is_globally_consistent`** — the "arbitrary trees"
    exit criterion: for a random rooted tree, if every direct parent→child
    edge is individually permitted, then no descendant outruns any ancestor
    and no internal node is archived. The local check composes into a sound
    tree-wide guarantee.
  - **`parse_accepts_exactly_the_known_actions`** + **`as_str_then_parse_round_trips`**
    — `parse` accepts exactly the three wire strings over random snake-case
    noise; `parse ∘ as_str` is the identity.
- **Dev-dep** — `proptest = "1"` (already used in `maidan-a2a` /
  `maidan-artifacts`). Tests only — no `src/` changes.

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| Cluster 113  | Backend parity harness | Next cluster of Phase XXI — a shared assertion suite both backends pass + a CI guard keeping `migrations/postgres` ↔ `migrations/sqlite` in lockstep. |
| Cluster 114  | Coverage-floor ratchet + fuzz | The floor bump (`COVERAGE_MIN_LINES` 11→25→40) and envelope fuzzing land as a dedicated step. |

## Surprises

- **No counterexamples — as expected.** The FSM is a 4-state linear machine
  with a monotone rank rule; proptest exploring the full `(state, action)`
  space and random trees found nothing, which is the *desired* result. The
  value is the regression guard: any future edge or rank change that breaks
  monotonicity or the tree-composition property now fails loudly.
- **Keeping `spec_next` independent from `apply` is what gives the table test
  teeth.** A test that re-uses `apply`'s own `match` would be tautological;
  an independent spec turns it into a genuine cross-check.

## Decisions

- **Assert invariants, not just the transition table.** The rank-advance and
  tree-composition properties hold even if the edge set changes shape, so they
  encode the *design intent* (monotone lifecycle, child never outruns parent)
  rather than the current edge list. No [[Architecture]] change.

## Capability table extension

| Capability | Where |
|------------|-------|
| FSM transition + rank invariants under arbitrary inputs | `maidan-fsm/tests/fsm_properties.rs` |
| Hierarchical (tree-wide) rank-rule guarantee | `maidan-fsm/tests/fsm_properties.rs` (`locally_valid_tree_is_globally_consistent`) |

## Risks identified + mitigated

- **Combinatorial blind spot.** Per-edge unit tests can't cover the full
  `(state, action)` space or arbitrary tree shapes; the property suite now
  does, closing the gap between "the three happy edges work" and "no illegal
  edge or rank violation is reachable".

## Risks identified + still open

- **Persistence/HTTP wiring** of the FSM (in `maidan-store` /
  `maidan-server`) is out of this crate's scope; its parity is part of
  Cluster 113's backend harness.

## Forward look

Phase **XXI** continues with **Cluster 113 — backend parity harness**: a
shared assertion suite both Postgres and SQLite backends must pass, plus a CI
guard that `migrations/postgres` ↔ `migrations/sqlite` and the store modules
stay in lockstep.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
