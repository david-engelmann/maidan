# Cluster 228.0 — task-schedule REST management API

> Program B (agentic orchestration), part 12. Phase XXIV post-gate hardening.
> Tag **`v228.0.0`**. No new gate tag.

## Goal

Give the scheduler (226 store, 227 worker) its outward CRUD surface: create / list /
pause-resume / delete schedules over REST, so an operator drives it without poking
the store — the REST half of the DAG-style REST/MCP split (229 is the MCP half).

## Scope

| Change | Where |
|--------|-------|
| `POST /workspaces/:wid/task-schedules` (create), `GET` (list), `PUT /task-schedules/:id` (pause/resume), `DELETE` | `routes/task_schedule.rs`, `app.rs` |
| `Store::set_task_schedule_active` (pause/resume), both backends | `store.rs`, `store/*/task_schedules.rs` |
| DTOs `CreateTaskSchedule` / `SetTaskScheduleActive`; full new-route preflight | `dto.rs`, `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Design decisions

- **Write = `workspace:write` + target-channel access.** A schedule spawns threads
  into `channel_id`, so creating / mutating one requires access to that channel
  (`ensure_channel_access`), not just the workspace — a private-channel schedule
  can only be managed by someone who can see the channel. List is `workspace:read`.
- **Create defaults are the intuitive ones.** `interval_secs` omitted → one-shot;
  `first_run_at` omitted → `now` (fires on the next sweep tick). So `{channel_id,
  title, interval_secs: 3600}` means "every hour starting now."
- **`created_by = auth.member_id`.** The acting member owns the schedule (audit).
  This is why the e2e runs with **auth enabled + a real minted token** rather than
  the bypass harness — bypass's nil member would violate the `created_by` FK (a real
  deployment always has a real member behind a token).
- **`PUT` for pause/resume**, one endpoint with `{active}`, rather than two verbs —
  fewer routes, same surface.

## Non-goals / deferred

- MCP tools (Cluster 229).
- Editing a schedule's interval/title/channel (delete + recreate for now).

## Risks

- **`created_by` under `AUTH_DISABLED`.** In the dev-only auth-disabled mode the
  caller is the nil member, so a create would FK-fail. That mode is fail-closed in
  prod (Cluster 157); real tokens always carry a real member. Noted, not a prod
  concern.
