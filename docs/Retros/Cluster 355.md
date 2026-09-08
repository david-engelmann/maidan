# Cluster 355 retro — the owner/steer cluster (W1)

Wave 1 #6 (W1 = G-dev-2 + G-dev-4). A task thread already had an *assignee* (the
claimer that does the work), but no one *accountable* for it — an auto-reviewer
subagent is not an owner. This cluster adds a durable **owner** axis, enforces
that the claimer cannot land its own owned work (separation of duties), persists
steering guidance across handoffs, and notifies the owner when an owned task
gets stuck.

## What shipped

- **355.1 (#642) — owner axis foundation.** `Thread.owner_id: Option<MemberId>`
  (pg 0059 / sqlite 0058, FK members `ON DELETE SET NULL`), `Store::set_thread_owner`
  (set/clear, both backends), orthogonal to the claim axis. Zero-blast-radius
  (159/217/230 pattern) — the full ~46-site column-list ripple + 5 `Thread`
  literals, no enforcement yet.
- **355.2 (#643) — owner REST + separation of duties.** `PUT`/`DELETE
  /threads/:id/owner`; and the "claimer cannot merge its own PR" rule enforced in
  the shared `transition_in_tx` (both backends) — on an owner-governed thread, a
  terminal transition by the assignee is rejected; the owner or another member
  must land it. Inert until an owner is set, so zero impact on existing
  transitions.
- **355.3 (#644) — persist steer.** `maidan_thread_steer` (pg 0060 / sqlite
  0059), `ThreadSteerStore` sub-trait, `PUT`/`GET /threads/:id/steer`. A durable
  steering instruction (latest wins) that survives claims and handoffs — distinct
  from a Cluster-195 handoff note (event-borne, not persisted).
- **355.4 (#645) — notify owner on stuck.** A `ClaimExpired` arm in the
  notification router: when an owned task's claim lapses and it's reclaimed, the
  owner gets a per-recipient notification (via the existing `notify` path, so it
  rides mute prefs + inbox + email). The event carries the `Thread`, so the owner
  is read with no extra fetch. Un-owned expiries notify no one.
- **355.5 (#646) — MCP surface.** `set_thread_owner` (set/clear) +
  `set_thread_steer` / `get_thread_steer`, the twins of the REST. SoD needs no
  MCP change — it lives in the shared transition core, which every claim/transition
  tool already routes through.

## Decisions

- **Separation of duties as one store-layer invariant.** The rule lives in
  `transition_in_tx` (shared by `transition_thread` + `transition_thread_with_event`,
  both backends), so REST *and* MCP *and* any future caller are covered by one
  check. It keys on `owner_id.is_some() && to_state.is_terminal() && actor ==
  assignee`, so an un-owned thread is completely unrestricted — setting an owner is
  the opt-in into governance.
- **evict-on-… reuse, not reinvention.** "Stuck" is exactly the Cluster-351
  `ClaimExpired` signal (a lapsed, reclaimed lease); "notify" is exactly the
  Cluster-238 `notify` helper. 355.4 is a single match arm bridging the two.
- **Owner + steer are separate tables/axes, not thread columns (steer) / a new
  column (owner).** Owner is a single member ref → a thread column (like
  assignee, with the attendant ripple). Steer is a per-thread payload with its
  own provenance (steered_by/at) → a side table (like `thread_results`), avoiding
  another `row_to_thread` ripple.

## Surprises

- **The owner column ripple hit ~46 SELECT/RETURNING sites** across
  `threads.rs` + `thread_transitions.rs` (both backends). A two-pattern sed keyed
  on the unique `assignee_id, assignment_expires_at, claim_lease_id,
  work_started_at` column-list signature (which never appears in a SET clause) did
  it safely; the full `cargo test -p maidan-store` (pg included) confirmed no miss.
- **A new `#[async_trait]` sub-trait needs the attribute on its declaration**,
  not just the impls, or E0195 (lifetime mismatch) — hit on `ThreadSteerStore`.
- **The `context_query_count_e2e` flake** (small=9 / large=8) reappeared on
  355.4's integration job — the known connection-warm-up flake; a rerun cleared
  it (memory `maidan-context-query-count-flake`).

## Test evidence

- `thread_owner` (both backends): owner set/clear + orthogonal-to-assign, the SoD
  suite (claimer's Close rejected, owner lands), the steer suite (set/get/upsert).
- `notification_router_e2e::router_notifies_the_owner_when_an_owned_task_gets_stuck`
  — owned expiry → the owner; un-owned expiry → no one.
- `owner_and_steer_tools_set_get_and_clear` (MCP) + `openapi_e2e` bijection +
  `http_capability_matrix_e2e` + `tools_catalog_contract`, all green per PR.

## Forward look

**W1 is complete** — a durable owner ≠ claimer, the claimer can't land its own
owned work, steering persists, and the owner is notified on stuck, over REST +
MCP. Next-ranked is **Wave 1 #7 — F1 + F2 + F7** (every post a titled thread;
collapsed children; leaf mute; `ThreadBumped`).

## Acknowledgements

Built as a five-PR run (#642 → #646) on the 351 occupancy clocks + the 238
notification router, each rebased/branched onto `main` as its parent merged.
