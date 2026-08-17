# Cluster 226.0 — scheduled/recurring task foundation

> Program B (agentic orchestration), part 10. Phase XXIV post-gate hardening.
> Tag **`v226.0.0`**. No new gate tag.

## Goal

Open the scheduled/recurring-task subsystem with a **zero-blast-radius foundation**:
the storage + model for a schedule that materializes a task thread when due, plus
the query the sweeper will use — and nothing wired in yet. Mirrors how the DAG
(217) and channel membership (159) started: land the table + store first, so the
next clusters (worker, REST, MCP) build on a tested base.

## Scope

| Change | Where |
|--------|-------|
| `maidan_task_schedules` table (pg 0038 / sqlite 0037), registered in `migrate.rs` | `migrations/{postgres,sqlite}/`, `migrate.rs` |
| `TaskSchedule` / `NewTaskSchedule` models + `TaskScheduleId` | `maidan-types/src/{models,ids}.rs` |
| 5 store methods, both backends: `create` / `get` / `list` / `delete` / `due` | `store.rs`, `store/{sqlite,postgres}/task_schedules.rs`, `store/*/mod.rs` |

## Data model

A schedule row: `{workspace_id, channel_id, title, interval_secs, next_run_at,
last_run_at, active, created_by, …}`.

- `interval_secs = NULL` → **one-shot**: fires once, then `active = false`.
- `interval_secs = Some(n)` → **recurring**: re-arm `next_run_at += n s` after each
  firing (the sweeper's job, a later cluster).
- The sweeper creates a thread titled `title` in `channel_id` when
  `active AND next_run_at <= now`. `due_task_schedules(now, limit)` is that scan
  (active, `next_run_at <= now`, oldest first, batch-bounded).

## Design decisions

- **Task == thread, again.** A schedule doesn't introduce a new entity — when due
  it materializes a *thread* (the same "task" the DAG/queue clusters operate on).
  The schedule row is just the recurrence spec + a title template.
- **Foundation only.** No worker, no routes, no events — so the blast radius is a
  new table + a new store module. Zero existing code paths change.
- **`interval_secs`, not cron (yet).** Interval recurrence covers the common
  "every N seconds/minutes/hours" case with a trivially-correct re-arm; a cron/RRULE
  spec can layer on later if needed, without changing the row shape much.

## Non-goals / deferred (the rest of the subsystem)

- **The sweeper worker** (Cluster 227): a background loop that scans `due`, creates
  the thread, and advances/deactivates the schedule — needs a **multi-replica-safe
  atomic claim-and-advance** (so two replicas don't double-fire a due schedule),
  analogous to `claim_next`'s `FOR UPDATE SKIP LOCKED` / serialized-writer CAS.
- **REST management** (Cluster 228) + **MCP tools** (Cluster 229).
- Pause/resume (`set_active`) — lands with the management API.

## Risks

- **Double-fire across replicas** — not possible yet (no worker); the atomic
  claim is the headline correctness concern for 227 and is called out above.
