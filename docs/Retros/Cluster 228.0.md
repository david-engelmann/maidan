# Cluster 228.0 retro — schedules become operable

> Tag **`v228.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 12.

## What shipped

- REST management for task schedules — create / list / pause-resume / delete —
  plus `Store::set_task_schedule_active`. An operator can now drive the scheduler
  end to end without touching the store.

## Surprises / decisions

- **`created_by` forced a real-auth e2e.** The create handler stamps the schedule
  with `auth.member_id`, and that column is a `NOT NULL` FK to members. The
  `for_tests` bypass harness authenticates as the **nil** member, which doesn't
  exist — so a bypass create would FK-fail. The fix was to write the e2e the way
  `channel_access_e2e` does: auth **enabled**, mint a real token for a real member,
  send `Authorization: Bearer`. That's strictly better — it exercises the real RBAC
  path (`workspace:write` + `ensure_channel_access`) instead of skipping it. The
  general lesson: a handler that persists `auth.member_id` can't be tested through
  the nil-member bypass.
- **Access follows the *target channel*, not just the workspace.** A schedule fires
  threads into `channel_id`, so managing it requires access to *that channel* —
  a private-channel schedule shouldn't be visible/editable by a workspace member who
  can't see the channel. So create resolves the channel and `ensure_channel_access`;
  pause/delete resolve the schedule then gate on its channel.
- **The new-route preflight, in full, four times.** Path stubs + `paths(...)` regs +
  three `components(schemas(...))` (the two DTOs + `TaskSchedule` as a body) +
  `http-capability-map` entries + the matrix test's body clauses (POST + PUT need a
  valid body or the extractor 400s before `cap()` 403s) + a `/task-schedules/`
  substitution branch. `openapi_e2e` (bijection) and `http_capability_matrix_e2e`
  both green confirm the wiring is complete — the checklist paid for itself again.

## Capability table extension

| Change | Where |
|--------|-------|
| Task-schedule REST CRUD + `set_task_schedule_active` | `routes/task_schedule.rs`, `store/*/task_schedules.rs` |

## Risks identified + still open

- **`created_by` under `AUTH_DISABLED`** — nil member → create FK-fails in the
  dev-only auth-disabled mode; fail-closed in prod, so not a real concern.
- **No schedule editing** — change interval/title by delete + recreate for now.

## Forward look

The MCP half of the split is next (229): `create` / `list` / `delete` / pause-resume
schedule tools, so an MCP-only agent can schedule its own recurring work. After
scheduling: capability registry + skill routing, then coordination waits +
structured results. Then Programs C (notifications & reach) and D (scale &
durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 227.0]].
