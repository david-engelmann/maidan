# Cluster 227.0 retro — schedules start firing

> Tag **`v227.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 11.

## What shipped

- A background scheduler sweeper (opt-in) that fires due schedules — atomically
  claiming and advancing each (multi-replica safe) and creating its task thread —
  plus the `claim_next_due_schedule` store method and a firing metric.

## Surprises / decisions

- **The atomic claim was the point of splitting 226 out.** 226 flagged the
  multi-replica double-fire hazard as 227's headline concern, and that's exactly
  where the design work went. `FOR UPDATE SKIP LOCKED` (Postgres) makes concurrent
  replicas grab distinct rows; SQLite's single-writer serialization makes the
  select-then-update transaction safe. Advancing the schedule *in the same commit*
  that selects it is what makes it a claim rather than a race.
- **At-most-once beat at-least-once, deliberately.** Claim-then-create means a crash
  in the window drops one firing; create-then-claim would risk duplicate task
  threads on a retry. For "spawn a task," a missed run is recoverable and a
  duplicate-task storm is not — so the claim commits first, and the window is
  documented, not engineered away.
- **Fire-once-per-tick killed the catch-up storm.** The tempting re-arm is
  `next_run_at += interval`, but a schedule that's been due for hours would then fire
  dozens of times in one drain to "catch up," creating a pile of threads. Re-arming
  to `now + interval` fires once and reschedules from now — a little drift, no storm.
  The right trade for a task scheduler (a precise cron would choose differently).
- **A thread is the only output — no new event kind.** Firing just calls
  `create_thread_with_event`, so a scheduled task is indistinguishable downstream
  from any other new thread and rides the existing DAG/claim/ready machinery.
  `ThreadCreated` *is* the "a scheduled task appeared" event.
- **A shared-store test trap.** `claim_next_due_schedule` scans **globally** (the
  sweeper isn't workspace-scoped), so when two test suites ran against one store, the
  first suite's leftover active schedule polluted the second's claim ordering — the
  2nd claim returned the wrong row. The fix was to have the first suite delete its
  schedule; the lesson is that global-scan store methods break the "different
  workspace = isolated" assumption that most of the suite relies on.

## Capability table extension

| Change | Where |
|--------|-------|
| Scheduler sweeper + `claim_next_due_schedule` + firing metric | `scheduler.rs`, `store/*/task_schedules.rs`, `metrics.rs` |

## Risks identified + still open

- **At-most-once firing** on a crash mid-fire — accepted (a dropped run beats a
  duplicate-task storm); no reaper.
- **Opt-in** — schedules never fire until `MAIDAN_SCHEDULER_TICK_SECS` is set; the
  management API (228) should make that discoverable.

## Forward look

The scheduler needs its outward surface next: REST management (228) — create / list /
delete / pause-resume schedules — then MCP tools (229), the DAG-style REST/MCP split.
After scheduling: capability registry + skill routing, then coordination waits +
structured results. Then Programs C (notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 226.0]].
