# Cluster 227.0 — scheduler sweeper worker

> Program B (agentic orchestration), part 11. Phase XXIV post-gate hardening.
> Tag **`v227.0.0`**. No new gate tag.

## Goal

Make the schedules from Cluster 226 actually fire: a background sweeper that, each
tick, materializes a task thread for every schedule that has come due — with
multi-replica firing safety as the headline correctness concern.

## Scope

| Change | Where |
|--------|-------|
| `Store::claim_next_due_schedule(now)` — atomic claim + advance, both backends | `store.rs`, `store/{sqlite,postgres}/task_schedules.rs`, `store/*/mod.rs` |
| Scheduler sweeper worker (opt-in `MAIDAN_SCHEDULER_TICK_SECS`) | `scheduler.rs`, `lib.rs`, `main.rs` |
| `maidan_task_schedules_fired_total{outcome}` metric | `metrics.rs` |

## Design decisions

- **Atomic claim-and-advance is the whole ballgame.** `claim_next_due_schedule`
  selects the oldest due schedule **and advances it in the same commit** — Postgres
  via a `FOR UPDATE SKIP LOCKED` CTE (concurrent replicas each grab a distinct row,
  never the same one), SQLite via a serialized select-then-update transaction. The
  worker creates the thread *after* that commit, so the ordering is **at-most-once**
  on crash (a dropped firing, never a duplicate) — the right default for "create a
  task": a missed run is recoverable, a storm of duplicate tasks is not.
- **Fire-once-per-tick, no catch-up storm.** A recurring schedule re-arms to
  `now + interval` (not `next_run_at + interval`), so a schedule that's hours overdue
  fires exactly once and reschedules from now, instead of firing dozens of times to
  "catch up." Slight drift, no thundering herd — the pragmatic choice for a task
  scheduler (vs. a precise cron). One-shots deactivate.
- **The thread is the output.** Firing creates a normal thread via
  `create_thread_with_event` (+ `publish_stored`), so it flows through the event
  stream and the whole DAG/claim/ready machinery — no new event kind, no special
  case. `ThreadCreated` is the "a scheduled task appeared" signal.
- **Opt-in, off by default.** Unset `MAIDAN_SCHEDULER_TICK_SECS` → the sweeper never
  starts (mirrors the retention sweeper). So the smoke/integration jobs and existing
  deployments are byte-unchanged until an operator enables it.
- **Per-tick firing cap (1000).** Belt-and-suspenders against a huge due backlog
  creating unbounded threads in one pass; the remainder fires next tick.

## Non-goals / deferred

- **REST management** (Cluster 228) + **MCP tools** (Cluster 229) — creating /
  listing / deleting schedules from outside; pause/resume.
- Claim leases / reaper for a worker that dies mid-thread-create (the at-most-once
  window is accepted).

## Risks

- **At-most-once firing** — a crash between the claim commit and the thread create
  drops that one run. Accepted + documented; the alternative (thread-first) risks
  duplicate tasks, which is worse.
