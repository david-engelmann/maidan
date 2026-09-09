# Cluster 358 retro — the budget envelope (Wave 1 #9, T1/T5)

Wave 1 #9 (T1/T5). Jevons: cheaper tokens buy more agent-hours, so an agent loop
with no ceiling is a runaway waiting to happen — and a run that quietly stops is
indistinguishable from one that succeeded. This cluster gives a task/run a
**budget envelope** (tokens, USD, turns, wall-clock) that **binds and stops the
run** when exceeded, records the stop as a **failure** (not a close), and
dead-letters it for triage.

## What shipped

- **358.1 (#658) — budget + usage store foundation.** `maidan_thread_budgets`
  (thread_id PK) side table (pg 0063 / sqlite 0062): optional
  `max_{tokens,usd_micros,turns,wall_secs}` + accumulated `used_*`. `ThreadBudget`
  / `BudgetLimits` / `UsageDelta` / `BudgetReason` + `ThreadBudget::exceeded(wall)`
  (pure). `BudgetStore` sub-trait: `set`/`get`/`add_thread_usage`. Zero-blast.
- **358.2 (#659) — `ClaimFailed` event + agent-work DLQ.** The failure
  vocabulary: `EventKind::ClaimFailed` (the full 11-site drill; non-federatable) +
  `maidan_agent_work_dlq` (pg 0064 / sqlite 0063) + `DlqEntry` / `NewDlqEntry` +
  `record_dlq_entry` / `list_channel_dlq`. Zero-blast.
- **358.3 (#660) — enforcement + REST.** `report_thread_usage` accumulates and,
  if the thread is now over budget AND has an active claim, atomically (one tx)
  releases the claim, appends `ClaimFailed`, and records a DLQ entry. REST:
  `PUT`/`GET /threads/:id/budget`, `POST /threads/:id/usage`,
  `GET /channels/:cid/dlq`.
- **358.4 (#661) — MCP.** `set_thread_budget` / `get_thread_budget` /
  `report_usage` / `list_dlq` — the MCP twins.

## Decisions

- **Budget-exhaustion is a claim-level failure, not a new terminal FSM state.**
  The item's wording (`ClaimFailed`, "agent-work DLQ") pointed here, and it avoids
  rippling a new `ThreadState` variant through the whole codebase. The thread
  keeps its lifecycle state; the *run* (claim) fails, the claim is released, and
  the work is dead-lettered — so it can be retried, re-budgeted, or abandoned. A
  hard stop is distinctly NOT a `Closed` (success).
- **USD as integer micros.** Money never touches a float in the schema or the
  wire.
- **Side table for the budget** (not `maidan_threads` columns) — keeps the hot
  `row_to_thread` path untouched (the `maidan-schema-column-ripple` lesson).
- **Wall time derives from the working clock**, not a stored countdown — the
  Cluster-351 `work_started_at` against `max_wall_secs`, checked at report time.
- **Enforcement at the heartbeat, not a reaper.** A well-behaved agent reports
  usage as it works and is stopped at the report that crosses the line; a silent
  hung agent is still caught by the existing lease-expiry reclaim (351). No new
  worker. Over-budget with no active claim is a no-op stop (nothing to fail).
- **Atomic stop.** The release + `ClaimFailed` append + DLQ insert commit in one
  tx with the usage write (the Cluster-205–214 outbox pattern) — a crash can't
  leave a released claim with no failure record, or a DLQ entry with no event.

## Surprises

- **`doc_lazy_continuation` twice.** A wrapped doc-comment line starting with
  `+ DLQ)` reads as a markdown list continuation under `-D warnings` (rust 1.91).
  Reworded both (REST + MCP) to prose. A wrap that starts a line with `+`/`(`/`)`
  is the trigger.

## Test evidence

- Store: `thread_budget` suite (both backends) — set/get/accumulate, the pure
  `exceeded` cases, DLQ record/list + channel isolation, and the enforce scenario
  (under → claim intact; over → claim released + `ClaimFailed` + DLQ;
  unassigned-over → no-op). Plus `exceeded()` unit tests in maidan-types.
- REST e2e `report_usage_over_budget_stops_and_dead_letters`; MCP
  `budget_tools_set_report_and_dlq`; the EventKind round-trip / federatable /
  contract tests cover `ClaimFailed`; openapi + capability-matrix + both MCP
  contract-sync tests green.

## Forward look

**The budget envelope binds and stops the run over REST + MCP**, with a DLQ for
triage and `ClaimFailed` as the "hard stop ≠ success" signal. **Deferred (logged
in Open Work):** a reactive `wait_for_claim_failed` long-poll (the `ClaimExpired`
analogue); wall-budget enforcement on the lease-reclaim path (a silent hung agent
is reclaimed, not budget-failed); a DLQ replay/retry action; and OpenMeter
entitlements → stop (this is the envelope that binds, deliberately NOT a billing
SKU). Next-ranked is **Wave 1 #10 — N2 / N5 / N4** (a digest of buried decisions;
an inbox grouped by thread + snooze; `during:` date-range search).

## Acknowledgements

Built as a five-PR run (#658 → #662) on the Cluster-351 claim/lease clocks, the
Cluster-205–214 transactional-outbox pattern, and the Cluster-355 owner axis,
each rebased onto `main` as its parent merged.
