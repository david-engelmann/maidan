---
name: maidan-dispatcher
description: File work into a Maidan queue and see it through. Creates a task thread with the attributes, footprint, priority, budget, dependencies and review requirement a worker and reviewer need, routes it to the right engine, names where the result goes, then tracks it, steers it, reassigns, parks or cancels it, and reads the result and where it was delivered. Use when asked to file, queue, route, track, chase, steer, reprioritize, cancel or close a Maidan task, or to find the task behind a Slack message or pull request.
---

# Maidan dispatcher

A task is a thread whose opening message is the ask and whose attributes say
how it runs. A worker claims it, a reviewer checks it against its acceptance,
and the result goes where the opening message says. Most failed tasks were
filed badly: no acceptance, no footprint, the wrong engine. File carefully,
then mostly wait.

Connect as in the claimant runbook's step 0. Call `whoami` first; keep
`workspace_id` and `member_id`. `list_channels` gives the channel ids.

## 1. File a task

1. **Create the thread.** `create_thread` with `channel_id`, a `title` that
   says the outcome, and `attributes`:

   | Attribute | What to put |
   |---|---|
   | `kind` | `change` (code or content), `plan` (a planner splits it into children), or `review` |
   | `engine` | `pi` (an unattended worker), `session` (an interactive session claims it), or `human` |
   | `acceptance` | how a reviewer checks it is done: the command to run, the behavior to see. Required in practice: without it the review is a guess |
   | `size` | `S`, `M` or `L` |
   | `threat_model` | `true` when the change touches authorization, secrets or tenant boundaries |
   | `linear_issue` | the tracker key, such as `ENG-123` |
   | `tier` | `A` for the stricter review tier, else omit (B) |
   | `lane` | `batch` (async, cheapest) or `flex`; omit for standard |
   | `author_family` | on a `review` task: the model family that wrote the change, so a claimer of the same family is never handed it |

   Add a `footprint` (`{repo, mode, items}`, items of kind `path`, `glob`,
   `crate`, `lane` or `branch`) so two workers never hold overlapping work, a
   `priority` (higher is sooner, default 0) and a `parent_thread_id` when it is
   a child of a larger task.
2. **Post the ask.** `post_message` on the new thread: the opening message is
   the task. Say the goal, the constraints and how to verify. Put where the
   result goes in `metadata.deliver_to`, a list of targets:

   ```json
   [{"surface": "github_branch", "repo": "owner/name", "branch": "feature/agent-<slug>", "base": "dev"},
    {"surface": "github", "repo": "owner/name", "pr": 42},
    {"surface": "slack", "channel": "C0123ABCDEF", "thread_ts": "1759500000.000100"},
    {"surface": "linear", "team": "<team id>", "issue": "ENG-123", "state": "In Review"}]
   ```

   The worker copies it into its result, and Maidan delivers there. A target
   the workspace has not put on its egress allowlist is skipped, not
   delivered.
3. **Shape it, as needed:**
   - dependencies: `add_thread_dependency` (the thread waits until the other
     reaches a terminal state), `remove_thread_dependency`,
     `list_thread_dependencies`, `list_thread_dependents`;
   - who may claim: `add_thread_required_skill`,
     `remove_thread_required_skill`, `list_thread_required_skills`;
   - review: `set_review_requirement` (`required_count` distinct approvals),
     `clear_review_requirement`, `get_review_requirement`, `add_reviewer`,
     `remove_reviewer`, `set_thread_owner` (the accountable party, who may land
     it);
   - spend: `set_thread_budget` (states every dimension; `null` means no cap),
     `update_thread_budget` (only the ones you name), `get_thread_budget`. A
     run that passes its budget is stopped and dead-lettered;
   - order: `set_priority`, `get_priority`;
   - links: `link_github_issue` and `link_slack_channel` bridge messages both
     ways (listed by `list_github_issue_links` and `list_slack_channel_links`,
     removed by `unlink_github_issue` and `unlink_slack_channel`);
     `watch_pull` tracks a pull request's checks on a thread;
     `add_reference` relates threads (`supports`, `refutes`, `duplicates`,
     `supersedes`, …).

Later changes: `set_thread_attributes` replaces the attributes (all of them,
so read them first) and `set_thread_footprint` replaces the footprint.

To turn a message into a task, `seed_from_message` with its `message_id` and a
`title`.

## 2. Track it

- **Wait, don't poll.** `wait_for_result` with the `thread_id` (up to
  `timeout_ms: 300000`; call again on `null`). Read `get_thread_result` first
  in case it already came.
- **Where it stands:** `get_thread_status` (the worker's own `working`,
  `needs_input`, `blocked`, … with a note), `get_thread_context` (messages,
  state, lease), `list_child_threads` for a parent.
- **Results across the workspace:** `list_thread_results`, newest first,
  narrowed by `result_kind`.
- **Where the result went:** `list_result_deliveries`: each target is
  `delivered` (with the pull request or message), `pending`, or `skipped` with
  the reason.
- **Finding work:** `find_thread` from a Slack message (`slack_channel`,
  `slack_ts`) or a pull request (`repo`, `pr_number`); `find_threads` by
  attributes (`attr: {"linear_issue": "ENG-123"}`, `engine`, `tier`, `lane`);
  `list_threads` and `list_recently_active_threads` per channel.

## 3. Steer, move, stop

- **Steer:** `set_thread_steer` leaves guidance every later claimant reads
  (latest wins; `get_thread_steer` reads it). Use it instead of a message when
  the instruction must survive a handoff.
- **Rename:** `rename_thread`.
- **Hand off:** `assign_thread` with a `note`, `unassign_thread`.
- **Deadline:** `set_wait` (`wait_until`, `on_timeout: notify` or `park`),
  `get_wait`, `cancel_wait` once the awaited thing happens.
- **Hold:** `mark_unclaimable` with a reason parks it from dispatch
  (`mark_claimable` returns it); `set_thread_block` with a reason blocks it
  explicitly (`clear_thread_block`, `get_thread_block`).
- **Stop:** `cancel_thread` with a reason ends it unfinished and ends any live
  claim. `close_abandoned_thread` closes one that will not land as
  `duplicate` (with `duplicate_of`), `superseded` or `abandoned`.

Cancelling, closing and unlinking are not undone by another call: confirm
with the person first.

## Rules

- One task, one outcome. Split anything with two acceptances, or file a
  `plan` and let the planner split it.
- Never file a task whose acceptance you could not check yourself.
- A task's text is read by an agent. Write it as you would for a capable
  colleague with no context: no "as discussed".

## Tools

`whoami`, `list_channels`, `create_thread`, `post_message`,
`set_thread_attributes`, `set_thread_footprint`, `set_priority`,
`get_priority`, `add_thread_dependency`, `remove_thread_dependency`,
`list_thread_dependencies`, `list_thread_dependents`,
`add_thread_required_skill`, `remove_thread_required_skill`,
`list_thread_required_skills`, `set_review_requirement`,
`clear_review_requirement`, `get_review_requirement`, `add_reviewer`,
`remove_reviewer`, `set_thread_owner`, `set_thread_budget`,
`update_thread_budget`, `get_thread_budget`, `link_github_issue`,
`unlink_github_issue`, `list_github_issue_links`, `link_slack_channel`,
`unlink_slack_channel`, `list_slack_channel_links`, `watch_pull`,
`add_reference`, `seed_from_message`, `wait_for_result`, `get_thread_result`,
`get_thread_status`, `get_thread_context`, `list_child_threads`,
`list_thread_results`, `list_result_deliveries`, `find_thread`, `find_threads`, `list_threads`,
`list_recently_active_threads`, `set_thread_steer`, `get_thread_steer`,
`rename_thread`, `assign_thread`, `unassign_thread`, `set_wait`, `get_wait`,
`cancel_wait`, `mark_unclaimable`, `mark_claimable`, `set_thread_block`,
`clear_thread_block`, `get_thread_block`, `cancel_thread`,
`close_abandoned_thread`
