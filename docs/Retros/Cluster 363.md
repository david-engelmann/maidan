# Cluster 363 retro — Unclaimable (Wave 1 #13 cont., G3)

Wave 1 #13 bundles wait-edges/`on_timeout` (G2), an escalation enum (G4), and
fair dispatch + Unclaimable + hard WIP (G3/G11). Cluster 362 shipped G11 (the WIP
limit); this cluster ships **G3 Unclaimable** — the ability to **park a thread
from dispatch** with a reason, distinct from blocked-by-deps, blocked-by-gate, and
skill-miss: an explicit human/owner park (needs triage, waiting on external,
broken). A parked thread stays open but `claim_next` skips it and an explicit
`claim` is refused, until it is un-parked.

## What shipped

- **363.1 (#690) — the store foundation (zero-blast).** `maidan_thread_unclaimable`
  (pg 0067 / sqlite 0066): a row (`thread_id` PK) marks the thread parked with
  `{reason, marked_by, marked_at}`; **presence = unclaimable, absence = claimable**.
  `ThreadUnclaimable` model + `AssignmentStore::mark_thread_unclaimable` (upsert) /
  `mark_thread_claimable` (clear → bool) / `get_thread_unclaimable` /
  `list_unclaimable_threads(channel)`. New `unclaimable.rs` in both backends
  (parity-clean).
- **363.2 (#692) — enforce in dispatch.** `claim_next`/`claim_next_with_event`
  (both backends, all 4 SQL sites) gain a `NOT EXISTS maidan_thread_unclaimable`
  clause beside the deps/skill/gate clauses. Queue-depth becomes a **4-way
  partition**: `QueueDepth` gains `unclaimable`, and both backends compute it while
  excluding parked threads from `ready`/`blocked`, so `open = ready + assigned +
  blocked + unclaimable` and `ready` stays exactly what `claim_next` takes.
- **363.3 (#694) — REST.** `PUT`/`DELETE /threads/:id/unclaimable` (park with a
  non-empty reason / un-park; 204/404) + `GET /channels/:cid/unclaimable` (parked,
  newest first); explicit `claim_thread` refuses a parked thread with **409**.
- **363.4 (#695) — MCP.** The same refusal on `claim_thread` + `mark_unclaimable` /
  `mark_claimable` / `list_unclaimable` tools.

## Decisions

- **Unclaimable MUST be in the `claim_next` SQL (unlike WIP).** WIP is about the
  *caller's* capacity — a boundary pre-check works. Unclaimable is about the
  *thread's* eligibility as a candidate — `claim_next` picks the oldest eligible
  thread, so a parked thread must be excluded from candidates in the query itself
  (a `NOT EXISTS` clause, like deps/skills/gates). The explicit `claim` path, which
  targets a specific thread, refuses via a boundary `get_thread_unclaimable` check
  (a 409/InvalidParams).
- **Parked = no claim, auto or explicit.** `claim_next` skips it; an explicit
  `claim` 409s. To work a parked thread, un-park it first — a clean, single
  semantics. (`renew_claim`/`acknowledge` on an already-held thread are unaffected;
  the park is about *new* dispatch.)
- **A side table, presence = state.** Consistent with the mute/follow/wip tables —
  a row means parked, no row means claimable; the reason/actor ride the row.
  Avoids the `row_to_thread` schema-column-ripple.
- **Queue-depth stays a total partition.** Rather than let `ready` over-count
  parked threads, a fourth `unclaimable` bucket keeps `open = ready + assigned +
  blocked + unclaimable` exact and `ready` = precisely what `claim_next` would take.

## Surprises

- **The `for_tests` nil-member trap (documented, bit again).** The park handler
  persists `auth.member_id` (the `marked_by` FK). `AppState::for_tests` disables
  auth, so the bypass `AuthContext` is the **nil member** (not in `maidan_members`)
  → the FK fails as a generic "database error" 500. A direct store call from the
  test's own task succeeded (real member), which isolated it to the auth path. Fix:
  the e2e builds `AppState::new(..., auth_disabled=false, ...)` and mints a real
  token (the channel-access/task-schedule pattern). Any handler persisting
  `auth.member_id` needs this.
- **A clippy SIGTERM (signal 15) on `workspace_purge_e2e`** during a
  concurrent-build burst read like a compile error but was a transient kill (the
  heavy-build flake class); a clean re-run passed.

## Test evidence

- Store: `thread_unclaimable` both backends (mark/remark-upsert/get/list +
  idempotent clear; `run_claim_skip_suite` — `claim_next` skips the older parked
  thread, the queue-depth bucket counts it, un-park restores claimability). Plus
  `thread_deps` (queue-depth literals), `assignment_readside`,
  `channel_queue_depth_e2e`, backend/dialect parity.
- REST: `unclaimable_e2e` (park / empty-400 / channel list / explicit-claim 409 /
  claim_next skip / un-park 204→404 / claimable). openapi bijection + capability
  matrix.
- MCP: `unclaimable_tools_park_a_thread_from_dispatch`; both contract-sync tests.

## Forward look

**Wave 1 #13 now has G11 (WIP, 362) + G3 Unclaimable (363).** **Still open (the
G2/G4 half, a distinct build):** wait-edges + `on_timeout` firing an escalation
*policy* — **`on_timeout` = no-decision / park, never an invented human refusal**
(the "TimedOut ≠ Decline" rule); steal the Restate/Temporal promise/timer *shape*,
not the engine. G3 fair dispatch (anti-starvation ordering; today oldest-first)
also remains. Then **Wave 1 #14** (N1/T6/H15/SCIM).

## Acknowledgements

Built as a four-PR run (#690 → #692 → #694 → #695) on the Cluster-217–225 DAG /
claim / queue-depth infrastructure + the Cluster-362 dispatch-safety pattern — each
rebased onto `main` as its parent merged.
