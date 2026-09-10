# Cluster 364 retro — wait-edges + on_timeout escalation (Wave 1 #13 cont., G2/G4)

The final piece of Wave 1 #13. With WIP (362) and Unclaimable (363) shipped, this
cluster lands **G2 wait-edges + G4 escalation**: a thread can declare a durable
**wait timer** with a deadline and an `on_timeout` escalation policy. Either it is
cancelled (satisfied — the awaited thing happened) or a background sweeper fires it
on timeout — reaching a human and optionally parking the thread. It steals the
Restate/Temporal promise/timer *shape*; it is **not** a workflow engine, and it
**never invents a decision**.

## What shipped

- **364.1 (#698) — the store foundation (zero-blast).** `maidan_thread_waits`
  (pg 0068 / sqlite 0067): one wait per thread — `wait_until` + `on_timeout` +
  reason + `created_by` + `fired_at`. `EscalationPolicy` enum (G4): `Notify` /
  `Park`. `set_thread_wait` (upsert, resets `fired_at`) / `cancel_thread_wait` /
  `get_thread_wait` / **`claim_next_due_wait`** — the atomic fire-once claim
  (`FOR UPDATE SKIP LOCKED` on pg).
- **364.2 (#701) — the `WaitTimedOut` event vocabulary.** Full EventKind drill,
  non-federatable; `policy` names the escalation applied.
- **364.3 (#700) — the sweeper + notification-router arm (the firing mechanism).**
  `wait_sweeper.rs` (opt-in `MAIDAN_WAIT_SWEEP_TICK_SECS`): drain due waits →
  `Park` marks the thread unclaimable (reuse 363) → publish `WaitTimedOut`; the
  router's `WaitTimedOut` arm notifies the thread's owner. `maidan_wait_timed_out_total`.
- **364.4 (#702) — REST.** `PUT`/`DELETE`/`GET /threads/:id/wait`.
- **364.5 (#703) — MCP.** `set_wait` / `cancel_wait` / `get_wait`.

## Decisions

- **`on_timeout` = no-decision / park, never an invented refusal.** The load-bearing
  constraint (the "TimedOut ≠ Decline" rule, docs/research-operator.md): a timeout
  must not fabricate a human's approval or refusal. So the escalation vocabulary is
  strictly **reach + park**: `Notify` emits `WaitTimedOut` (the router reaches the
  owner); `Park` additionally marks the thread unclaimable so `claim_next` won't
  dispatch a stuck thread until a human intervenes. Neither closes/approves/declines.
- **Steal the shape, not the engine.** A wait is a durable one-shot timer on a
  thread — a row with a deadline that is either cancelled or fired by a sweeper.
  No orchestration DSL, no promise graph; the existing DAG (deps) and gate (350)
  cover the other "wait" shapes. This is the *timer with escalation* shape.
- **Fire-once is atomic and idempotent.** `claim_next_due_wait` claims + stamps
  `fired_at` in one statement (pg `FOR UPDATE SKIP LOCKED`; sqlite serialized tx),
  so every replica running the sweeper never double-fires one wait. A re-set is a
  fresh timer (`fired_at` reset to NULL).
- **The sweeper decouples from notification.** The sweeper *fires* (park + emit
  `WaitTimedOut`); the notification router — a separate bus consumer — *reaches*
  the owner off that event. Same split as `ClaimExpired` (355).
- **`Park` reuses Unclaimable (363).** A stuck thread's escalation is exactly the
  "park from dispatch" primitive shipped last cluster — no new mechanism.

## Surprises

- **The `for_tests` nil-member FK trap, a third time.** Both `set_thread_wait`
  (REST + MCP) persist `auth.member_id` (`created_by` FK). `for_tests` disables
  auth → the bypass nil member → a generic "database error" 500 / FK failure. The
  REST e2e builds `AppState::new(..., auth_disabled=false)` + a minted token; the
  MCP test uses a real `from_session` member. (The sweeper e2e is fine on
  `for_tests` — the sweeper uses the wait's stored `created_by`, a real member, not
  an auth context.)

## Test evidence

- Store: `thread_waits` both backends (set/get/cancel + re-set-resets-fired +
  `claim_next_due_wait` fires only the due one, once, stamps `fired_at`);
  backend/dialect parity.
- Types: the event round-trip / federatable / uniqueness suite + `event_kinds_contract`.
- Server: `wait_sweeper_e2e` (a due `Park` wait → one firing → thread parked with
  the reason + `WaitTimedOut{policy:"park"}` → a second sweep fires nothing → the
  router notifies the owner); `wait_rest_e2e`; openapi bijection + capability matrix.
- MCP: `wait_tools_set_get_cancel`; both contract-sync tests.

## Forward look

**Wave 1 #13 is now COMPLETE** — G11 WIP (362), G3 Unclaimable (363), and G2/G4
wait-edges + escalation (364). **Deferred (logged):** the `wait_for_wait_timeout`
MCP long-poll (the observation is already covered by the notification + the
`/mcp/stream` subscribe on `kinds=wait_timed_out`); a richer G4 escalation
vocabulary (the research mentions TAKEOVER/SPAWN_REVIEWER/PAUSE — each is more
decision-adjacent and a larger design); and **G3 fair dispatch** (anti-starvation
`claim_next` ordering; today oldest-first) — the last open sub-item of #13. Next
is **Wave 1 #14** (N1 web-push / T6 legal-hold / H15 OTel gate / SCIM-as-OIDC-P3 —
four bullets, not one cluster).

## Acknowledgements

Built as a six-PR run (#698 → #701 → #700 → #702 → #703) on the Cluster-227
scheduler-sweeper + Cluster-238/355 notification-router + Cluster-363 unclaimable
patterns — each rebased onto `main` as its parent merged.
