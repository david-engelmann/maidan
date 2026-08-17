# Cluster 226.0 retro — the scheduler gets a place to live

> Tag **`v226.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 10.

## What shipped

- The `maidan_task_schedules` table + `TaskSchedule` model + five store methods
  (create/get/list/delete/due, both backends). A schedule that will materialize a
  task thread when due — storage and the due-scan only, no worker or routes.

## Surprises / decisions

- **Foundation-first paid off before, so again.** The DAG (217) and channel
  membership (159) both started by landing the table + store with zero wiring, and
  the follow-up clusters were calmer for it. A scheduler is a genuinely new
  subsystem (a background loop, recurrence math, multi-replica firing safety), so
  splitting the storage out first keeps each PR small and the risky part (the
  worker's atomic claim) isolated to its own cluster.
- **No new entity — a schedule makes a *thread*.** The whole task-queue arc
  (217–225) is built on "task == thread." A schedule is just a recurrence spec plus
  a title; when it fires it creates a thread, which then flows through the existing
  DAG / claim / ready machinery. That kept the model tiny (one table) and means the
  scheduler inherits everything the queue already does.
- **Interval, not cron.** `interval_secs` (NULL = one-shot, `Some(n)` = every n s)
  covers the common case with a re-arm so simple it's obviously correct
  (`next_run_at += n`). Cron/RRULE can layer on later without reshaping the row.
- **The migration checklist is muscle memory now.** New `.sql` × 2 backends, a
  `const include_str!` + an `apply_{pg,sqlite}(pool, N, …)` per backend in
  `migrate.rs` (pg one ahead: 0038 / 0037), a new `TaskScheduleId` in `ids.rs`, and
  the store module registered in both `mod.rs`. All caught at compile/test time; the
  full `cargo test -p maidan-store` (not a subset) is the guard.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_task_schedules` + `TaskSchedule` model + CRUD/due store methods | `migrations/*`, `models.rs`, `store/*/task_schedules.rs` |

## Risks identified + still open

- **Multi-replica double-fire** — the headline correctness concern for the *worker*
  (Cluster 227), not yet present (no worker). The sweeper will need an atomic
  claim-and-advance (the `claim_next` `FOR UPDATE SKIP LOCKED` / serialized-writer
  pattern) so two replicas can't both fire one due schedule.

## Forward look

The subsystem builds out next: the background sweeper (227, with the atomic claim),
then REST management (228) and MCP tools (229) — the same REST/MCP split as the DAG
clusters. After scheduling: capability registry + skill routing, then coordination
waits + structured results. Then Programs C (notifications & reach) and D (scale &
durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 225.0]].
