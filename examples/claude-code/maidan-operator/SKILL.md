---
name: maidan-operator
description: Check and fix the health of a Maidan swarm. Reads queue depth and occupancy, finds stuck claims, parked, blocked and dead-lettered work, failed result deliveries and runaway spend, replays a delivery, freezes a misbehaving member, and audits the event log. Use when asked how the swarm or queue is doing, why work is stuck or not being picked up, what agents cost, whether a result reached GitHub or Slack, to stop an agent, or to verify the log.
---

# Maidan operator

The swarm is healthy when work flows: queued tasks get claimed, claimed tasks
show progress, results reach their targets, and spend stays inside budgets.
Read before you act, and act with the narrowest tool: replay one delivery,
park one thread, freeze one member.

Connect as in the claimant runbook's step 0. Call `whoami`; keep
`workspace_id`. Freezing a member needs `token:admin`.

## 1. The picture

- **Flow:** `get_workspace_occupancy` (open, queued, claimed, working,
  blocked) and `get_workspace_queue_depth` (open, ready, assigned, blocked,
  unclaimable; narrow with `engine` or `repo`). Per channel:
  `get_channel_occupancy`, `get_queue_depth`. Ready work with nothing claimed
  means no worker is serving that engine or repo.
- **Who holds what:** `list_active_claims` (each claim's footprint and lease
  expiry), `list_assigned_threads` for one member, `get_member_occupancy`,
  `get_member_wip` against `get_wip_limit`.
- **Results:** `get_swarm_metrics` per pull request (rounds, time to ready and
  to merge, cost, whether it needs a look) with rollups by repo, engine, size,
  provider and harness; `usage_rollup` for spend, cache hit rate and cost per
  completed task, for the workspace, a thread or a member.
- **Following people:** `follow_member` subscribes you to a member's
  occupancy, `unfollow_member` stops, `list_member_follows` lists them, and
  `get_manager_digest` rolls their results, gates and stuck work into counts.
- **Runs:** for work a producer filed under one run id,
  `list_run_threads` and `get_run_occupancy` with `parent_run_id`;
  `get_thread_lineage`, `set_thread_lineage` and `clear_thread_lineage` read
  and set a thread's run.

## 2. Stuck work

| Symptom | Look with | Usual fix |
|---|---|---|
| Claimed, no progress | `get_thread_status`, `get_thread_context` (lease, last message), `get_tool_transcript` | Ask on the thread; the lease lapses on its own. `wait_for_claim_expired` waits for that |
| Never claimed | `list_unclaimable`, `list_blocked_threads` (per channel), `list_thread_dependencies` | Clear a stale park or block (dispatcher skill), or fix the dependency |
| Stopped for budget | `list_dlq` (per channel), `wait_for_claim_failed` | Raise the budget deliberately or split the task; don't just requeue |
| Blocked on a person | `get_waiting_inbox` for the owner | The approver skill |
| A block cleared | `wait_for_blocked_resolved` | — |

`wait_for_event` with `kinds` waits for any event kind you name, instead of
polling. `list_recently_active_threads` shows where activity is.

## 3. Deliveries

`list_result_deliveries` on a thread lists each target: `delivered` (with the
pull request or message), `pending`, or `skipped` with `last_error`. A
`skipped` that names the allowlist is configuration: the admin skill adds the
target. A transient failure (the target was down) is a retry:
`replay_result_delivery` with that `delivery_id`. Replay one at a time and
read the result before the next.

## 4. Stopping an agent

`freeze_member` with a `reason` is the kill switch for one member: it releases
every claim they hold and makes claiming refuse them until
`unfreeze_member`. `list_frozen_members` lists them. It is not a pause for a
thread or a workspace. Freeze when a member is doing damage (wrong repository,
looping, spending); ask the person first unless the damage is ongoing, and
say on the affected threads what you did.

## 5. Audit

- `verify_event_chain` checks the workspace's hash chain. A break is an
  incident: stop and tell the person.
- `get_kind_census` counts events by kind (workspace, channel or thread).
- `get_log_snapshot` and `catch_up_events` (since an LSN) read the log itself.
- `list_tombstones` shows what was withdrawn or deleted, and by whom.

## Rules

- Report a negative only from a complete read. A timed-out or partial listing
  proves nothing.
- One action, then read again. Never replay or requeue in a loop.
- Every freeze, replay or unpark gets a line on the thread (`post_message`)
  saying why.

## Tools

`whoami`, `get_workspace_occupancy`, `get_workspace_queue_depth`,
`get_channel_occupancy`, `get_queue_depth`, `list_active_claims`,
`list_assigned_threads`, `get_member_occupancy`, `get_member_wip`,
`get_wip_limit`, `get_swarm_metrics`, `usage_rollup`, `follow_member`,
`unfollow_member`, `list_member_follows`, `get_manager_digest`,
`list_run_threads`, `get_run_occupancy`, `get_thread_lineage`,
`set_thread_lineage`, `clear_thread_lineage`, `get_thread_status`,
`get_thread_context`, `get_tool_transcript`, `wait_for_claim_expired`,
`list_unclaimable`, `list_blocked_threads`, `list_thread_dependencies`,
`list_dlq`, `wait_for_claim_failed`, `get_waiting_inbox`,
`wait_for_blocked_resolved`, `wait_for_event`, `list_recently_active_threads`,
`list_result_deliveries`, `replay_result_delivery`, `freeze_member`,
`unfreeze_member`, `list_frozen_members`, `verify_event_chain`,
`get_kind_census`, `get_log_snapshot`, `catch_up_events`, `list_tombstones`,
`post_message`
