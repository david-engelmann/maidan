# Running an agent swarm on Maidan

Maidan is the coordination layer for a swarm of AI coding agents: one queue of
tasks that many agents claim from, without stepping on each other, with every
result reviewed and delivered back to where the work came from. The agents can
be any MCP client, an A2A agent, or your own loop over REST. Maidan holds the
coordination, and the agents hold none of it.

This page is the shape of a swarm built on Maidan, and the pieces it uses. The
tool-by-tool reference is [Integration](Integration.md); result delivery is
[Result Delivery](Result%20Delivery.md).

## Why the coordination lives outside the agents

Studies of swarms with tens of concurrent agents keep finding the same things:

- **Locks the model manages fail; coarse claims the harness holds work.** An
  agent that forgets to release, or reasons its way past a lock, takes the
  whole swarm with it. A claim with a lease that expires on its own does not
  depend on the agent behaving.
- **Concurrent agents on one repository conflict often**, commonly a fifth to
  two fifths of their changes. The fix is to keep them on separate parts of
  the code, decided before they start, not merged afterwards.
- **The bottleneck moves to review and merge.** Ten agents writing code need
  something that tells you which ten results are ready, and what each one did.
- **Everything an agent reads is untrusted input**: issue bodies, review-bot
  comments, other agents' messages. Coordination state that agents can only
  read and claim through narrow calls is safer than a shared file they edit.

Maidan is built around those findings.

## The loop

```
issue tracker ──▶ thread (task) ──▶ claim ──▶ work ──▶ result ──▶ review ──▶ delivered
   intake          with metadata     lease      isolated   envelope   gates      PR, issue
                   and footprint     + fence    worktree               + humans   comment, chat
```

1. **Intake.** A producer turns an issue into a thread: the task text, a
   `deliver_to` list saying where the result goes, and, when it knows, a
   footprint saying what part of the repository the work touches.
2. **Planning.** A task with `kind: plan` goes to a planner, which claims it
   and submits its children in one call: `POST /threads/{id}/plan` or the MCP
   `submit_plan`, with the claim's `lease_id` and 1–50 children. Each child has
   a key, a title, attributes, a footprint, a `test` command, and optionally a
   description and `depends_on` (keys of other children in the plan).
   - **Lint.** So each child can be worked and checked without asking the
     planner, every child needs `attributes.acceptance`, `attributes.engine`,
     a `test` command (kept at the end of its description as ``Test: `…` ``)
     and a footprint with at least one item. A child with `threat_model: true`
     goes to a session (`engine: session`). A child that misses any of these
     is a 400 naming it, and nothing is created.
   - **Held for review.** A plan with five or more children, or any child
     with `threat_model: true`, is created held: the answer says
     `"held": true`, and each child carries a `human` block, so `claim_next`
     skips it and the board shows the hold. A Maintainer releases it with
     `POST /threads/{id}/plan/release` (`token:admin`, audited), which clears
     only the plan hold and answers the children released.
   - **All or nothing.** The children, their footprints, attributes and
     dependency edges are made in one transaction in the plan's channel. A bad
     child, an unknown or cyclic `depends_on`, a repository missing from the
     workspace's `github_branch` egress allowlist, or a plan larger than the
     spawn budget is a 400 naming the child, and nothing is created.
   - **Retries.** The same claim sending the same keys again gets a 200 with
     the children already made. Any other plan on a parent that has children
     is a 409 (`plan-conflict`, MCP `-32010`) listing them.
   - **Dispatch.** `claim_next` never hands out a thread whose dependencies
     are unfinished, so a child becomes claimable when what it depends on
     lands. A plan's children move through review and close while the plan
     stays open under its planner's claim.
   - **A child that closes without landing** (no `ThreadLanded`: closed by
     hand, or cancelled) blocks each open dependent with a `dag` block naming
     it, plus a message on the dependent, so no work starts on a base that
     never merged. The plan gets one message. A person or the planner clears
     the blocks once the work is redone.
   - **The plan closes itself** when its last child lands, in that child's
     transaction, through the plan's own review and close gates, with a
     `ThreadLanded` and a message listing every child's pull request. When a
     gate refuses, the plan stays open with one message saying why.
   - `GET /threads/{id}` on a plan shows `plan_state: {total, closed, open,
     blocked}`.
3. **Claiming.** Workers call `claim_next_thread` (or the workspace-wide
   `claim_next_workspace_thread`). Each claim carries a lease and a fencing
   token (`claim_lease_id`). The worker acknowledges it when it starts
   (`acknowledge_claim`), renews it while it works (`renew_claim`), and
   releases it if it stops early (`release_claim`). A lease that lapses
   returns the thread to the queue, charged against its budget, so a crashed
   agent never holds work.
4. **Footprints.** The claim queue only hands out work whose footprint
   overlaps no live claim (below).
5. **Work.** Each agent works in its own worktree or container. Maidan does
   not run the agents.
6. **Result.** The agent sets a structured result (`set_thread_result`). A
   change result names the branch, the base commit and the diff. Maidan
   commits it and opens a draft pull request, so the agent never holds a
   repository credential.
7. **Review.** Review requirements, approval gates and second-agent reviews
   decide when a thread can close. An agent cannot approve its own work.
8. **Delivery.** The result is delivered to every target the producer named,
   once the workspace has allowed it: a GitHub pull request or comment, a
   Slack thread, a Linear issue comment (with an optional state move). See
   [Result Delivery](Result%20Delivery.md).
9. **Merge queue.** Once checks and reviews pass, the client that marks agent
   pull requests ready can enqueue one (`POST /operator/github/enqueue`): Maidan
   adds the repository's merge-queue label, and only when the pull request's
   head is still the commit Maidan delivered, so nothing pushed after review
   reaches the queue. While a pull request is queued, Maidan refuses to push
   another change to it; dequeue first (`DELETE`). The queue itself (a label
   driven one, such as Graphite's) does the merging. A repository a person
   merges is left out of `MAIDAN_MERGE_QUEUE_LABELS` and is never enqueued.

## Claim footprints

A footprint declares what part of a repository a thread's work touches:

```json
{
  "repo": "example/app",
  "mode": "exclusive",
  "items": [
    { "kind": "lane",   "value": "bridge" },
    { "kind": "glob",   "value": "src/bridge/**" },
    { "kind": "branch", "value": "feature/agent-bridge-retry" }
  ]
}
```

- **Kinds.** `lane`, `crate` and `branch` match by equal value. `path` and
  `glob` share one namespace and match by prefix, a glob counting up to its
  first wildcard: `src/bridge/**` overlaps `src/bridge/retry.rs` and `src/`,
  not `src/gateway/`. A path never conflicts with a crate, so name the lane
  too when you want that safety.
- **Modes.** An `exclusive` footprint conflicts with any overlap. A `shared`
  one conflicts only with an exclusive one. A `branch` is always exclusive:
  one agent per branch.
- **Scope.** Overlap is checked only within one workspace and one repository.
  `repo` is `owner/name`, and a GitHub clone URL is accepted.
- **Lifetime.** A footprint counts only while its thread holds a live claim,
  and frees itself when the claim ends by a result, a release or a lapsed
  lease. Nothing has to clean it up.

The calls:

| | REST | MCP |
|---|---|---|
| Declare at intake, with the thread (`workspace:write`) | `POST /channels/{cid}/threads` with `footprint` | `create_thread` with `footprint` |
| Refine or replace (no items clears; `thread:transition`) | `PUT /threads/{id}/footprint` | `set_thread_footprint` |
| Read one | `GET /threads/{id}/footprint` | (in the thread's context) |
| The live claims and their footprints | `GET /workspaces/{id}/claims?repo=` | `list_active_claims` |
| The threads waiting, and why (overlap, paused repository, full slot) | `GET /workspaces/{id}/deferred` | — |

**How they fit together.** A producer that knows the lane declares the
footprint in the call that creates the thread. The two commit together, so
the queue applies the footprint from the first claim, and the producer needs
only the `workspace:write` it creates threads with, not the `thread:transition`
a worker holds. The queue then defers that thread until the lane is free; a deferred thread counts as blocked in the queue
depth, not ready. A worker that learns more after claiming refines the
footprint. When the refined footprint overlaps another live claim, the call
is refused (`409` with problem type `footprint-conflict`, or MCP error
`-32009`), nothing is stored, and each overlapped claim comes back with its
holder and `lease_expires_at`, so the worker can release its claim and requeue
after that lease would end.

**A cheap win.** Declaring `{"kind": "branch", "value": <the agent branch>}`
on every change task gives one agent per branch with no other setup.

### Workers bound to one repository

A worker whose checkout is one repository passes `repo` when it claims
(`claim_next_thread` or `claim_next_workspace_thread` over MCP, or the
`claim-next` routes' body, in any spelling a footprint accepts). It is then
handed only threads whose footprint names that repository, so it never claims
work it would have to release. A thread without a footprint is never handed
to a worker that names a repository, so producers declare `repo` at intake.

### Build slots

Compiling and testing bind long before tokens do, so a swarm on one machine
admits work by build capacity. A footprint item `{"kind": "slot", "value":
"rust-build"}` names a shared resource instead of part of the repository.
Slots are counted across the whole workspace, whatever the repository or
mode, and their names compare case-insensitively.

| | REST |
|---|---|
| Set a capacity (`token:admin`) | `PUT /workspaces/{id}/slots` with `{"slot": "rust-build", "capacity": 3}` |
| Remove it (`token:admin`); `404` when it had none | `DELETE /workspaces/{id}/slots?slot=rust-build` |
| List capacities and use (`workspace:read`) | `GET /workspaces/{id}/slots` |

With a capacity of N, `claim_next` hands out a thread holding the slot only
while fewer than N live claims hold it. The rest wait in order, counted as
blocked, and the next one goes out when a holder finishes, releases or lets
its lease lapse. A refinement onto a full slot is refused like an overlap,
naming the slot's holders. A slot without a capacity limits nothing but is
still listed with its use, so an operator can see the load before capping
it. Setting and removing a capacity are audited (`slot.capacity.set`,
`slot.capacity.clear`).

Pick slots for what actually contends: one per heavy toolchain (`rust-build`,
`docker`), sized to the cores and memory each build needs. Lanes stay for
what must not be edited twice; slots are for what must not run too many
times at once.

### Pausing a repository

An operator can stop the swarm taking new work in one repository (during a
deploy freeze, a broken `main`, or an incident) without stopping the rest:

| | REST |
|---|---|
| Pause (`token:admin`), with an optional reason | `PUT /workspaces/{id}/repo-pauses` with `{"repo": "example/app", "reason": "deploy freeze"}` |
| Resume (`token:admin`); `404` when it was not paused | `DELETE /workspaces/{id}/repo-pauses?repo=example/app` |
| List (`workspace:read`) | `GET /workspaces/{id}/repo-pauses` |

While a repository is paused, `claim_next` defers every thread whose footprint
names it, exactly as it defers an overlapping one: the thread keeps its place
and counts as blocked. Claims already held run on, so work in flight finishes.
A thread with no footprint is never paused, so a producer that wants the
pause to apply declares `repo` when it creates the thread, not after the claim.
A pause binds one workspace, and both the pause and the resume are audited
(`repo.pause`, `repo.resume`).

### Change notices

At three to five workers the pain is stale bases: one agent's change merges
over files another is still editing, and the second finds out in review.
Maidan tells it first, with a **notice** on its thread: a `ThreadNotice`
event and one message saying the same in a sentence.

- **`rebase_needed`.** When the change sweep sees an agent pull request
  merged, it takes the files every round of it touched (each round records
  its diff's paths, at most 500) and looks for live claims in the same
  workspace and repository whose footprint covers one, by the footprint
  rules: a `path` or `glob` item matches by prefix; lane, crate, branch and
  slot items say nothing about files and never match. Each such thread gets
  one notice naming the repository, the pull request, the base branch and
  the covered paths (at most 20). A merge notifies once, whatever the sweep
  does after; another workspace's claims are never told.
- **`stale_base`.** When the change flow delivers a round whose base is
  `MAIDAN_STALE_BASE_COMMITS` (default 50) or more commits behind its branch,
  that thread gets a notice, once per delivered commit.

A worker reads them on its next turn: the last 10, newest first, are the
`notices` of `GET /threads/{id}` and of the context pack (MCP
`get_thread_context`, `GET /threads/{id}/context`), in the pack's tail so the
cached prefix does not change. Maidan does not rebase on its own; the worker
does. When a queue takes only linear pull requests, the worker rebases its
checkout and sends a **squash-rebase round** (`"rebase": true`, the whole
branch's diff against the base's current head): Maidan rebuilds the branch as
one commit there and moves it with its one guarded force write, refusing if
anyone else pushed to the branch, the base moved on, or the pull request is in
the merge queue. A rebase that stops on conflicts comes back as
`rebase_conflict`, which writes nothing and names the paths in the reply
([Result Delivery](Result%20Delivery.md)).

## Cancelling a task

A producer that no longer needs a task (the plan changed, the issue closed, a
duplicate turned up) cancels it rather than waiting for the worker to finish:

| | REST | MCP |
|---|---|---|
| Cancel, with a reason of 1 to 500 characters | `POST /threads/{id}/cancel` with `{"reason": "superseded"}` | `cancel_thread` |

Every thread records the member that created it (`created_by`), on every
create path: REST, MCP `create_thread`, a seeded thread, an A2A conversation,
a recipe and a schedule. A cancel needs `workspace:write`: with it a member
may cancel a thread it created, and with `thread:transition` as well any
thread it can reach. Anyone else gets `403`.

In one transaction the cancel:

- closes the thread. It goes straight to `closed`, around the review, land
  and parent gates, because those guard landing work and a cancel lands
  nothing;
- parks it from dispatch (the unclaimable mark, with the reason), so nothing
  reopens it into the queue;
- ends any live claim. The assignee and lease are cleared and the worked time
  is charged, so the worker's next `renew_claim` is refused (`404`), and it
  should stop;
- appends `ThreadStateChanged` and `ThreadCancelled` (`reason`,
  `cancelled_by`), and posts the reason on the thread as a message whose
  metadata is `{"notice": "cancelled"}`.

A second cancel answers `200` with the first one and changes nothing. A thread
that closed or was archived some other way answers `409`. A cancel publishes
no `ThreadReady` for the thread's dependents, but `claim_next` still counts a
cancelled dependency as finished, as it does any closed one.

## Task attributes

A thread can carry attributes that say what kind of work it is, who or what
should do it, and whether it needs a threat model:

```json
{ "kind": "change", "engine": "pi", "threat_model": false,
  "size": "M", "acceptance": "The bridge retries twice; its test proves it." }
```

`kind` is `change` (the default), `plan` or `review`. `engine` is `pi`,
`session` or `human`, or absent when nobody has routed the work yet.
`threat_model` marks work that needs one. `size` (`S`, `M` or `L`) is the
planner's estimate, and `acceptance` says how a reviewer verifies the task;
the worker sees it in the thread context. The lane is not an attribute: it is
a footprint item.

A producer sets them, and a dispatch `priority` (higher is claimed first),
in the call that creates the thread, under the `workspace:write` it creates
threads with.

They are set at intake, on `POST /channels/{cid}/threads` or MCP
`create_thread`, in the same transaction as the thread, and replaced later
with `PUT /threads/{id}/attributes` or MCP `set_thread_attributes`
(`thread:transition`). They ride on `GET /threads/{id}`, on the thread
context (`thread.attributes`, REST and MCP), on the `claim_next` answer
(REST and MCP, channel and workspace), and on each `list_active_claims`
entry. A thread nobody described
reads as a change with no engine and no threat model.

A claimant waits for work without polling: MCP
`claim_next_workspace_thread` takes `wait_ms` (up to 300000). With nothing
claimable it waits for work to become claimable (a new thread, a dependency
finishing, a claim released or expiring, a thread reopened) and claims it,
or answers null at the deadline. A session claimant calls it in a loop with
`engine: "session"` and a long `wait_ms`, and needs no event stream of its
own.

Claiming respects the engine. `claim_next` (HTTP and MCP, channel and
workspace) takes an optional `engine`, and a claimer that names one is
handed only work routed to it. A Pi worker that passes `engine: "pi"` is
never handed session or human work, and work nobody routed waits for a
claim that names no engine.

A workspace can route that work by default. Maidan creates threads that name
no engine: schedules fire them, inbound A2A messages open them, a message
becomes one, and a plan child or a REST or MCP create can leave the engine out.
`PUT /workspaces/{id}/default-engine` with `{ "engine": "pi" }` (`token:admin`,
audited as `workspace.default_engine.set`) routes every thread created in the
workspace afterwards with no engine to `pi`, in the transaction that creates
it, so a Pi worker's `claim_next` with `engine: "pi"` is handed it. An engine
named at creation still wins, threads created before keep their routing, and
`{ "engine": null }` turns the default off. `GET` on the same path reads it
(`workspace:read`).

A worked pass through the surface: intake files a review for a Pi worker
and a plan for a session,

```http
POST /channels/{cid}/threads
{ "title": "review the claim queue change",
  "attributes": { "kind": "review", "engine": "pi", "threat_model": true } }

POST /channels/{cid}/threads
{ "title": "plan the metering rollout",
  "attributes": { "kind": "plan", "engine": "session" } }
```

The Pi worker's `claim_next` with `{ "engine": "pi" }` is handed the
review; the plan waits for a session claim. If the review turns out to
need a person instead, `PUT /threads/{id}/attributes` with
`{ "kind": "review", "engine": "human", "threat_model": true }` reroutes
it whole, and the metrics read for the workspace
(`GET /workspaces/{id}/swarm-metrics?engine=pi`) counts only the work
that was routed to Pi.

### Claim and release comments on Linear

A task's attributes may name the Linear issue it is for
(`"linear_issue": "ENG-1234"`; refused unless it is an issue key). A
workspace can then say each task's claim and release on that issue. The
switch is **off by default**: `PUT /workspaces/{id}/linear-claim-comments`
with `{"enabled": true}` (`token:admin`, audited) turns it on, and `GET`
reads it (`workspace:read`). While it is off, nothing is said on Linear.

With it on, the change sweep says each of these once per task:

- `CLAIM: pi/<task id> claimed in Maidan by <handle>: <title>` when the task
  is first claimed;
- `RELEASED: pi/<task id> ended without merging: <title>` when, after a
  claim was said, the task closes or is archived with no merged pull
  request.

The prefix is the task's engine (`maidan` when it has none). The comments go
out through the Linear delivery path with the same key and the same
per-team allowlist as a result's comment. There is no assignee and no new
identity. The issue is looked up first, and a comment is queued only when its
team is blessed for the workspace. An issue in another team, or one that
cannot be found, is recorded as skipped and never written to. The worker
checks the allowlist and the team again when it posts. A notice is recorded
in the same transaction that queues it, so it is said once even with
several replicas sweeping.

### Handoff at `non_converging`

A Pi task that stops converging is handed to a session, once. The change
sweep flags a task `non_converging` when it is still open after the round
limit (3), or its diff grew in each of the last three rounds. For a task
routed to Pi, it then:

- ends the claim, charging its worked time as any claim end does, so the
  worker's next renew is refused;
- sets the task's engine to `session`, so a Pi `claim_next` never takes it
  again, and leaves the task open for a session;
- appends a `ThreadHandoff` event (rounds, repository, pull request, the new
  engine), posts a line in the thread, and queues the same line in the
  result's Slack thread when that channel is blessed;
- where the workspace says claims on Linear, queues the handoff packet on
  the task's Linear issue (its `linear_issue` attribute, else the one its
  change result named). The packet gives the pull request and its head,
  each round with its diff lines and base lag, the cost, the open findings
  from the result, and the models the result reported. The release comment
  (`RELEASED: … handed off to a session after N rounds`) follows.

The swarm metrics count a handed-off task as misrouted, and each task says
`handed_off`.

### Session pull requests

The change flow records the pull requests it opens. A session opens its own,
so to compare engines a session's pull request is registered on its task:
`PUT /threads/{id}/pull` with `{"repo": "owner/name", "pull_number": N}`
(`workspace:write` and thread access), or the MCP tool `register_thread_pull`
with the same fields and `thread_id`. The repository must be blessed for the
workspace, for GitHub comments or agent branches. A task with no thread yet
gets one first, through `create_thread` with `engine: "session"` and its
`linear_issue`. From then on, the change sweep refreshes the pull request
like any other, and its merge closes the task. Its time to ready runs from
the task's `CLAIM:` comment (else from `claimed_at` in the body, for a
claim said outside Maidan, else from registration) to the first sweep
that sees the `ready-to-merge` label. It has no rounds, so its round and
edit numbers say nothing about it.

### CI on tracked pull requests

The change sweep reads each open tracked pull request once a tick, in one
GitHub GraphQL query, and the checks on its head come with that read. A pull
request is tracked when the change flow opened it, when a session registered
it (above), or when someone asks to watch it:

| | REST | MCP |
|---|---|---|
| Watch a pull request (`workspace:write`) | `POST /workspaces/{id}/pull-watches` with `{"repo": "owner/name", "pull_number": N}` | `watch_pull` |

The repository must be blessed for the workspace, as for a session pull
request, because the sweep reads it with the instance's GitHub credential. A
pull request the workspace already tracks answers with its thread
(`"created": false`). Otherwise Maidan makes it a thread, owned by the
watcher, in the `channel_id` given or in the workspace's public
`pull-requests` channel (made when missing), so its notices have a home. A
watched pull request is not an agent task: the swarm metrics and the outcome
checks leave it out. One another workspace tracks is a `409`.

Each read records `ci` on the thread (`GET /threads/{id}` and the context
pack's tail): the head commit, `status` (`pending` until every check has
finished, `failure` as soon as one fails, else `success`), the failing checks
(`failing_jobs`, at most 20), and the last 4 KB of the first failing GitHub
Actions job's log (`log_tail`), read once when a head first fails. It also
gives the thread these notices, each once per head commit, with a message
saying the same:

- **`pr_opened`**, the first time the sweep sees the pull request open (once
  per pull request; it names the head it was first seen at).
- **`ci_failed`**, when a check on the head fails, naming the failing checks.
- **`checks_done`**, when every check on the head has finished, with its
  `status`.

A push is a new head, so its checks are news again; the next tick over the
same head says nothing. A pull request that has closed is not read again.

The same read says what else holds the pull request back, and `ci` records
it: GitHub's `merge_state`, the unresolved review threads (a review bot's
count too: `unresolved_threads`, and links to the first 20 with who opened
them), and `missing_checks`, the checks the base branch's protection requires
that never reported though every check that did has finished. Together they
make `gate_reasons`: one sentence each for unresolved threads, a merge state
of `BEHIND`, `BLOCKED`, `DIRTY` or `DRAFT`, and each missing check. When the
reasons change, the thread gets **`gate_blocked`** with the new `reasons`, or
**`gate_clear`** when none are left; a read that says the same says nothing.
Reading the protection needs a credential that may read the repository's
administration settings; without it no check counts as missing.

`ci.merge_queue` follows the pull request through the base branch's merge
queue: `ready_to_merge` (it carries the `ready-to-merge` label and is not
queued), `queued`, `dequeued` with GitHub's `reason` (`MERGE_CONFLICT`,
`FAILED_CHECKS`, ...), and `landed` once its merge is recorded. Entering the
queue is said as **`merge_queued`**, and leaving it without merging as
**`merge_dequeued`** with the reason in `reasons`. A landing is the change
sweep's usual one: the thread closes with `ThreadLanded`.

A pull request closed without merging counts as landed only when its head
had commits its base lacked while it was open and is on the base branch now,
as when a stack tool lands the commits itself and closes the pull request.
The sweep asks GitHub once, when it first sees the close. A branch reset to
its base and closed was never ahead, so it lands nothing. Any other close is
a close without a merge, said once and leaving the task open.

### Deploys

A deploy pipeline can tell Maidan where a merged change went. It is opt-in,
and the route is a 404 until `MAIDAN_DEPLOY_HOOK_SECRET` is set. The
pipeline posts to `POST /integrations/deploys/events`, signing the body as
GitHub signs its webhooks (`X-Maidan-Signature-256: sha256=<hex
HMAC-SHA256(secret, body)>`):

```json
{"repo": "owner/name", "sha": "<full 40-character commit>",
 "environment": "production", "status": "success",
 "url": "https://deploys.example/42"}
```

`status` is `success` or `failure` (`error` counts as a failure); any other
status, such as `in_progress`, is acknowledged and says nothing, because only
an outcome is news. `url` is optional and must be `https://`. Each tracked
pull request in `repo` that merged at `sha` (its merge commit or its merged
head) gets one **`deployed`** notice on its thread. The notice carries
`environment`, `status` and `deploy_url`, in every workspace that tracks the
pull request and in no other. The same deploy and outcome said again (a
retried or replayed call) says nothing, so the signature needs no replay
window. The answer is `{"noticed": n}`. A deploy that arrives before the
change sweep has read the merge is not matched; send it again later.

## Measuring the ramp

`GET /workspaces/{id}/swarm-metrics` (`workspace:read`; `?repo=`, `?since=`,
`?rounds_limit=`, `?behind_limit=`, `?engine=`, `?kind=`), or the MCP tool
`get_swarm_metrics` with the same filters as arguments, gives, for each agent
pull request the change flow opened:
- its rounds (commit, diff lines, how many commits its base was behind,
  whether it was a squash-rebase, and for one, `previous_head`, the head it
  moved the branch from), and `rebases`, how many were;
- time to ready and time to merge;
- whether it merged at the commit Maidan delivered (`merged_without_edits`);
- the thread's model cost from the usage ledger;
- two flags.

`non_converging` means still open after the round limit (3), or a diff that
grew in each of the last three rounds (a squash-rebase round carries the
whole branch, so its size never counts as growth): reroute it to a session. `stale_base`
means a round was built on a base 50 or more commits behind.

Each repository gets a rollup: tasks, merged, merged without edits,
misrouted, the misroute rate, mean rounds and cost per merged change, then
closed unmerged, flagged and median time to ready. A task is **misrouted**
when the engine it was routed to could not finish it: it is open and flagged
`non_converging`, or it closed unmerged after reaching the round limit, or
it was handed to a session at `non_converging`.

The first seven numbers are repeated for three splits: by the engine each
task was routed to (`by_engine`: `pi`, `session`, `human`, then unrouted),
by size (`by_size`: `S`, `M`, `L`, then unsized) and by risk tier
(`by_risk`: standard work, then `threat_model` work). A split leaves out a
slice with no tasks. Pi is given small and medium work and sessions the risky
work, so compare engines within one size or one tier: across the whole
repository the comparison mostly measures the routing. Cost per merged change
in each engine slice is what compares Pi with sessions on cost.

Two more splits compare who did the work: `by_provider` (the model provider
that spent the most on each task: `anthropic`, `openai`, `xai`, ...) and
`by_harness` (the harness that ran it: `claude-code`, `cursor`, `codex`, ...),
each most tasks first with unnamed tasks last. Both come from the claimant's
own usage reports, so a claimant names them in `report_usage`'s `evidence`
(`{"provider": "xai", "harness": "openrouter"}`); a task whose usage named
none lands in the unnamed slice. Each task carries its `provider` and
`harness`, and `?provider=` and `?harness=` narrow the whole response to one.

Each task carries its attributes and, when known, its `linear_issue`: the
Linear key the change result names (`"linear_issue": "ENG-1234"`, ignored
unless it is a key), else the issue of its Linear `deliver_to` target. The
first round that knows it records it. `?engine=`, `?kind=` and `?size=`
narrow the whole report to one slice, and `?linear_issue=ENG-1,ENG-2`
to the tasks for those issues (the attribute, else the one the result
named), so a paired pilot can be read apart from other work in the same
repository. These are the numbers each ramp step is gated on.

The read refreshes up to 25 open pull requests from GitHub first, so merges
count on a stack that receives no webhooks.

**Whether a merge held.** For 30 days after a merge, the change sweep checks
each merged pull request at most once a day, in one batch per workspace an
hour, so the reads stay small against the token's hourly budget. A batch
reads the base branch's recent commits and its closed pull requests once per
repository and base, and matches every due pull request against those two
lists:

- `reverted` and `revert_ref`: a pull request titled `Revert "…"` that names
  it (`Reverts owner/repo#N`, or `(#N)` inside the quoted title), or a commit
  on the base whose message says `This reverts commit <merge commit>`;
- `followup_fixes`: later merged pull requests into the same base, within the
  30 days and not reverts, whose title or body says it fixes or follows up
  this one (`fixes #N`, `follow-up to #N`, its URL), or names its
  `linear_issue`;
- `human_commits`, `human_reviews` and `time_to_first_human_review_secs`
  (from ready, else from opened), read once after the merge:
  - human commits are the commits on the pull request that the change flow
    did not deliver;
  - human reviews are the reviews and review comments by accounts that are
    not bots (a pending review is not counted).

GitHub cannot time a person's effort, so these are proxies, not hours.

Each slice adds `reverted` and `followup_fixed`, with `revert_rate` and
`followup_fix_rate` over the merged tasks whose outcome is known (something
was found, or the 30 days have passed), plus `mean_human_commits` and the
median time to the first human review. Each list read takes the newest 100,
so a repository that merges more than 100 pull requests into a base within an
hour's batch can miss some.

## Scaling notes

- **Admit by build capacity, not by agent count.** Compiling and testing bind
  long before tokens do. Give heavy work a `slot` item and the slot a
  capacity (above), run as many workers as you like, and let the rest of the
  queue wait.
- **Keep lanes coarse.** A lane per crate or area that can change in parallel
  is enough; a footprint per file costs more than it saves.
- **Keep the claim lease short and renew it.** A lease of a few minutes,
  renewed while working, frees a dead agent's work quickly.
- **Measure merged-and-not-reverted per task**, not tasks closed. Maidan
  records each thread's usage (`report_usage`) and the queue's occupancy
  (`get_workspace_occupancy`), so cost per merged change is a query.

## What Maidan does not do

It does not run agents, schedule containers, or hold a build cache. It holds
the queue, the claims, the reviews, the evidence and the delivery, which is
the part a swarm has to share.
