# Cluster 368 retro — Wave 2 #16: the waiting-on-you inbox

Wave 2 #16 (G15 + G9) is a single, focused primitive: **what is waiting on *you*
right now** — assigned tasks, open approval gates, unread mentions — one member's
queue, each item aged against an SLA. Not `@everyone`, not a firehose.

## What shipped

- **368.1 (#726) — the aggregate + REST.** A pure `assemble_waiting_inbox` in
  `maidan-types` (`WaitingKind` / `WaitingItem` / `WaitingInbox`): it drops terminal
  and tombstoned assigned threads, merges the three sources, sorts oldest-waiting
  first, and flags `overdue` (age > sla). `GET /members/:id/waiting?sla_secs=N`
  composes the three existing store reads (`list_assigned_threads` +
  `list_pending_approval_gates` + `list_mentions_for_member` filtered to unread via
  the inbox cursor) and the assembler — **no new store code**.
- **368.2 (#727) — the MCP tool.** `get_waiting_inbox` — the same composition over
  the shared pure assembler, so an MCP-only agent can pull its queue.
- **368.3 (#728) — the `/ui` surface.** A "Waiting on you" section at the top of the
  Work tab, oldest-first with an overdue flag and a tunable SLA.

## Decisions

- **A pure assembler, not a store aggregate.** The three sources (threads / gates /
  mentions) are heterogeneous tables; a UNION query would be three dialect-specific
  SQL joins. Composing them in a pure function keeps the interesting logic
  (exclusion + sort + SLA) fully unit-testable and lets REST and MCP share it
  verbatim — the route/tool is just three reads plus the call.
- **Unread mentions only.** A member's whole mention history isn't "waiting on
  them"; only mentions after their inbox read-cursor are. The cursor
  (`get_inbox_last_read_at`) returns a bare `DateTime` defaulting to the epoch, so a
  never-read inbox keeps everything — no special-casing.
- **Gates are workspace-wide.** A pending approval gate needs *a* human, not a
  specific one, so every member's inbox surfaces the workspace's open gates. That's
  the G15 intent (open gates need attention), and it's cheap.
- **Self-only for a session, act-as-any for a bearer.** `ensure_acting_member`: a
  human sees their own queue; an orchestrator bearer can query any member's — the
  same model as the notification inbox (239). The `/ui` resolves the member from the
  session or the actor id, so the bearer Playwright fixture can drive it.

## Surprises

- **The inbox cursor is not an `Option`.** I wrote a `match Some/None` around
  `get_inbox_last_read_at` and it wouldn't compile — it returns a bare `DateTime`
  (epoch default), so the unread filter is a plain `created_at > last_read`.
- **Fresh test items are never overdue.** The e2e seeds items at `now`, so with any
  positive SLA they age to ~0s and none are overdue — the overdue math is proven by
  the pure unit test's aged fixtures (a 2h-old thread against a 1h SLA), not the e2e.

## Test evidence

- `maidan-types`: `waiting_inbox` unit tests (exclusion of terminal/tombstoned, the
  sort, the overdue flag, the empty case).
- Server: `waiting_inbox_e2e` (an assigned thread + a pending gate → total=2, both
  kinds, default SLA, fresh items not overdue).
- MCP: `waiting_inbox_tool_composes_assigned_and_gates`.
- `/ui`: `ui_js_wires_waiting_inbox` static guard + `waiting.spec.ts` Playwright.

## Forward look

**Wave 2 #16 is complete.** Deferred: a real SLA-breach *notification* (the router
could emit when an item goes overdue); per-source SLAs (a gate's deadline vs a
task's); a `wait_for_waiting` long-poll. **Next: Wave 2 #17** — an AG-UI door on the
existing WS/SSE (H1).

## Acknowledgements

Three stacked PRs (#726 → #727 → #728) plus this retro, on the pure-assembler + the
`/ui`-Work-tab (Cluster 367) patterns.
