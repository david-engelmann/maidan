# Cluster 350 retro — the held gate (durable human approval)

> Tag **`v350.0.0`**. Phase XXIV (post-gate hardening). **Wave 1 #1 of the
> forward program** ([[Open Work]]) — the first `NEW` row (F3 + F9 + H2 + H10 + N6).
> No new gate tag.

A **multi-PR cluster** (350.1–350.8 + this retro), shipped as a stacked-PR cascade
(each sub-PR CI-verified against `main` on its own, then the next rebased onto the
merged parent). Replaces the blocking MCP elicitation model with a **durable,
queryable approval gate**: an agent asks for human approval, gets an
async *input-required* handle, and the run is not held hostage to an open
socket — the human answers later over REST or the `/ui`, and the answer is
integrity-checked.

## What shipped

- **350.1 (#612) — durable approval-gate foundation.** A `maidan_approval_gates`
  table (pg 0056 / sqlite 0055) + `ApprovalGate`/`ApprovalGateState`/`NewApprovalGate`
  models + `ApprovalGateId` + store CRUD (create / get / list_pending / **resolve**).
  `resolve` is a compare-and-set on `state='pending'` (`WHERE id=? AND
  state='pending' RETURNING …`) — the concurrency primitive that makes "silence is
  not consent" enforceable. Zero-blast-radius foundation (159/217/226 pattern).
- **350.2 (#613) — `request_approval` returns input-required, no blocking.** The MCP
  `request_approval` tool now *creates a gate* and returns an MRTR
  `input_required` result immediately, instead of issuing a blocking
  `elicitation/create` over the session socket. `get_approval_gate` reads one back.
- **350.3 (#614) — REST answer + queryable list.** `GET
  /workspaces/:wid/approval-gates` (`workspace:read`) lists pending gates so an
  agent (or a human) can *find* them; `POST /approval-gates/:id/answer`
  (`workspace:write`) resolves one. The gate's `requestState` is **HMAC-SHA256
  signed** (`sign_request_state` / constant-time `verify_request_state`, keyed on
  the session-resume secret) — the token is untrusted and its integrity MUST be
  verified. `answer` maps a CAS miss to **409 Conflict** (already resolved), a bad
  `action` to **400**, and a bad state transition to **403** — so a double-answer,
  a garbage action, and an out-of-order answer are each distinct, honest failures.
- **350.4 (#615) — retire the deprecated sampling + roots tools.** `summarize_thread`
  (sampling) and `list_roots` (roots) — the two tools built on the server→client
  request path — are removed; both were deprecated in `rmcp` and neither had an
  organic caller.
- **350.5 (#616) — remove the dead `request_client` subsystem.** With sampling +
  roots gone, the whole server→client `request_client` machinery
  (`resolve_client_response`, `client_capability_for_method`, the per-session
  `pending` oneshot map, `client_capabilities`/`next_request_id` on the session) was
  unreachable, so it's deleted; `handle_in_session` collapsed into `handle`. **The
  `2024-11-05` protocol version and `Last-Event-ID` resumability were KEPT** — a
  deliberate scoping call (see Surprises): those are old-but-valid ways to use the
  tool, not dead code.
- **350.6 (#617, N6) — required-human approval as a claim gate.** A pending
  approval gate on a thread now blocks `claim_next` (a `NOT EXISTS (… g WHERE
  g.thread_id = <cand> AND g.state='pending')` clause beside the deps/skills
  clauses). "Required human" is a *claim gate*, not a notification preference — an
  agent won't be handed work that is waiting on a person.
- **350.7 (#618) — Playwright `/ui` test harness.** A seed-and-serve Rust example
  (`ui_test_server`) boots an in-memory SQLite server with a seeded
  workspace/member/channel/thread/gate + bearer, and a `ui-tests/` Playwright
  project (`@playwright/test`, headless Chromium) drives the real `/ui` in CI (a new
  `ui-tests` job). We no longer hand-test UI changes — the suite asserts it.
- **350.8 (#619) — the Approvals tab (the elicitation client).** A new "Approvals"
  tab in `static/index.html` lists pending gates and answers them (accept / decline
  / cancel) over the `/ui/api` twins of the 350.3 routes. `/ui` is the human
  elicitation client the held-gate model needs.

## Decisions

- **Kept `2024-11-05` + `Last-Event-ID` resumability.** The maintainer's rule
  (see [[Decisions]] / memory) is: clean up *dead or unreachable* code, but keep an
  old-but-valid approach someone might use for a simpler mental model. The
  `request_client` subsystem was genuinely dead (no caller after sampling/roots
  retired) → removed. The older protocol version and the resumable-stream path are
  live, supported ways to use the transport → kept.
- **HMAC over the request-state, not a DB round-trip on every read.** The
  `requestState` handed back to the agent is signed, not trusted; verifying it is a
  constant-time MAC check, so a tampered handle is rejected without a store lookup.
- **CAS, not a status read-then-write.** `resolve`'s `WHERE state='pending'` is the
  only thing standing between two racing answers; an empty accept, a re-answer, and
  a stale answer are all rejected by it (→ 409). "Silence is not consent" and "empty
  accept is not yes" are enforced here, not in the handler.

## Surprises

- **The `/ui` had never been browser-tested.** 350.7 is the first real UI test in
  the project — before it, the only `/ui` guard was `ui_js_contract` (a static
  undefined-helper grep). Playwright in CI (headless Chromium, seed-and-serve
  harness) closes that; the `ui-tests` job passed on its own PR (~4 min).
- **Retiring sampling/roots cascaded further than expected.** Removing two tools
  (350.4) made an entire transport subsystem dead (350.5) — a good illustration of
  "delete the caller, then the machinery falls out," but it needed a careful pass to
  confirm nothing else reached `request_client`.

## Test evidence

`approval_gate_e2e` (HMAC sign/verify, CAS 409, 400/403 mapping); `approval_gate_claim`
(a pending gate blocks `claim_next`, both backends); `ui_approvals_e2e` (the `/ui/api`
answer path); the `ui-tests` Playwright job (smoke + approvals specs, headless
Chromium in CI); both MCP contract-sync tests; full `cargo test -p maidan-store`
(both backends) + fmt + strict clippy + `--all-targets` + bootstrap-strip clean
across all eight PRs.

## Forward look

The held gate is the first `NEW` row of Wave 1 ([[Open Work]]). The natural
follow-up is **H12 — the HITL "list" remainder** (A2A `tasks/list` conformance so an
*external* agent can discover a pending gate: `input-required` status, page tokens,
`statusTimestampAfter`, `includeArtifacts`, `application/a2a+json`, `pageSize` max
100) — a gate no external agent can find is only half-built. G4 (an escalation enum)
comes *after* this gate, per the ranking.

## Acknowledgements

Solo maintainer cluster; stacked-PR cascade + admin-merge per [[Operations]]. Closes
Wave 1 #1 of the forward program ([[Open Work]]).
