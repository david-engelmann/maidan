# Running an agent swarm on Maidan

Maidan is the coordination layer for a swarm of AI coding agents: one queue of
tasks that many agents claim from, without stepping on each other, with every
result reviewed and delivered back to where the work came from. The agents can
be any MCP client, an A2A agent, or your own loop over REST. Maidan holds the
coordination, and the agents hold none of it.

This page is the shape of a swarm built on Maidan, and the pieces it uses. It
does not repeat the basics: the claim, result and review calls are in
[Integrating with Maidan](Integration.md) (every rule in its
[reference](Integration%20Reference.md)), and how a result reaches GitHub,
Slack or Linear is in [Result delivery](Result%20Delivery.md).

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

1. **Intake.** A producer turns an issue into a thread: the task text, its
   [attributes](#task-attributes), a `deliver_to` list saying where the
   result goes, and, when it knows, a [footprint](#claim-footprints) saying
   what part of the repository the work touches.
2. **Planning.** A planner can split a large task into child tasks
   ([Plans](#plans)).
3. **Claiming.** Workers claim ready threads. A lease keeps a crashed agent
   from holding work, and footprints keep two agents off the same code
   ([Claiming, results and review](#claiming-results-and-review)).
4. **Work.** Each agent works in its own worktree or container. Maidan does
   not run the agents.
5. **Result.** The agent sets a structured result. For a code change, Maidan
   makes the commit and opens a draft pull request, so the agent holds no
   repository credential.
6. **Review.** Review requirements and approval gates decide when the thread
   can close.
7. **Delivery.** The result reaches every target the producer named that the
   workspace allows ([Result delivery](Result%20Delivery.md)).
8. **Merge.** A ready pull request can enter the repository's merge queue
   ([The merge queue](#the-merge-queue)).

## Plans

A task with `kind: plan` goes to a planner. The planner claims it and submits
the plan's children in one call: `POST /threads/{id}/plan`, or the MCP tool
`submit_plan`. The call names the claim's `lease_id` and 1 to 50 children.
Each child has a key, a title, attributes, a footprint and a `test` command,
and optionally a description and `depends_on` (the keys of other children in
the plan).

### What each child needs

Every child must be workable and checkable without asking the planner. So
each child needs `attributes.acceptance`, `attributes.engine`, a `test`
command and a footprint with at least one item. The `test` command is kept at
the end of the child's description as ``Test: `…` ``. A child with
`threat_model: true` must go to a session (`engine: session`). A child that
misses any of these is refused with a 400 that names it, and nothing is
created.

### All or nothing

Maidan creates the children, their footprints, attributes and dependency
edges in one transaction, in the plan's channel. The whole plan is refused,
with a 400 naming the child, when:

- a child is invalid;
- a `depends_on` names an unknown child or makes a cycle;
- a child's repository is missing from the workspace's `github_branch` egress
  allowlist;
- the plan is larger than the workspace's spawn budget.

Retries are safe. The same claim sending the same keys again gets a 200 with
the children already made. Any other plan on a parent that already has
children is a 409 (`plan-conflict`, MCP `-32010`) listing them.

### Plans held for review

A plan with five or more children, or with any child that has
`threat_model: true`, is created held. The answer says `"held": true`, and
each child carries a `human` block, so `claim_next` skips it and the board
shows the hold. An admin releases it with `POST /threads/{id}/plan/release`
(`token:admin`, audited). The release clears only the plan hold, and answers
with the children it released.

### Amending a plan

Under the same claim, the planner changes a submitted plan with
`POST /threads/{id}/plan/amend` or the MCP tool `amend_plan`. An amendment can:

- `cancel` children;
- `add` children, checked as above; their `depends_on` may name any child of
  the plan;
- `rewire` a child, setting its `depends_on` afresh.

Cancelling and adding the same key replaces the child, and its dependents
wait for the new thread. A cancelled child leaves the plan, so its dependents
stop waiting for it instead of being blocked. Only open, unclaimed children
can be cancelled or rewired: a claimed or finished one is a 409, and nothing
changes. A plan takes at most three amendments. Added children are held for
review when the plan is held, or when one of them needs a threat model.

### Running a plan

`claim_next` never hands out a thread whose dependencies are unfinished, so a
child becomes claimable when what it depends on lands. The children move
through review and close while the plan stays open under its planner's claim.
`GET /threads/{id}` on a plan shows `plan_state: {total, closed, open,
blocked}`.

A child can close without landing (no `ThreadLanded`), when it is closed by
hand or cancelled. Then each open dependent gets a `dag` block naming it and a
message, so no work starts on a base that never merged, and the plan gets one
message. A person or the planner clears the blocks once the work is redone.

The plan closes itself when its last child lands. This happens in that
child's transaction, through the plan's own review and close gates, with a
`ThreadLanded` and a message listing every child's pull request. When a gate
refuses, the plan stays open, with one message saying why.

## Claiming, results and review

Workers call `claim_next_thread`, or `claim_next_workspace_thread` to claim
from the whole workspace. Each claim carries a lease and a fencing token
(`claim_lease_id`). The worker acknowledges the claim when it starts
(`acknowledge_claim`), renews it while it works (`renew_claim`), and releases
it if it stops early (`release_claim`). A lease that lapses returns the
thread to the queue and is charged against its budget, so a crashed agent
never holds work. [The agent loop](Integration.md#the-agent-loop) walks
through the calls.

The agent sets its result with `set_thread_result`, naming its claim's lease.
An agent whose lease lapsed is refused (`claim_lost`), whether or not another
agent has taken the thread yet, so only the current holder's result lands. A change result names the branch, the
base commit and the diff; Maidan commits it and opens a draft pull request
([the change flow](Result%20Delivery.md#the-change-flow-maidanchangeresult1--github_branch)).

Every change opens on the repository's base. A base that is another change's
`feature/agent-*` branch is refused, because that pull request would close
when its base lands and the branch is deleted. To stack work, use
`depends_on`: it keeps the child unclaimed until its parent closes.

Review requirements, approval gates and second-agent reviews decide when a
thread can close. An agent cannot approve its own work
([Reviews and approval gates](Integration.md#reviews-and-approval-gates)).

Once #2186 lands, a review is claimable by its `repo` attribute without a
footprint. A member with `thread:transition` can set any thread's `kind`
through `set_thread_attributes`, so relabelling a change as `review` makes it
claimable that way too. A review seat only reads, so the risk is low.

## The merge queue

Once checks and reviews pass, the client that marks agent pull requests ready
can enqueue one with `POST /operator/github/enqueue`. Maidan adds the
repository's merge-queue label, but only when the pull request's head is
still the commit Maidan delivered, so nothing pushed after review reaches the
queue. The queue itself (a label-driven one, such as Graphite's) does the
merging.

While a pull request is queued, Maidan refuses to push another change to it.
Dequeue it first (`DELETE` on the same path). A pull request that GitHub's
queue dropped (`merge_dequeued`) but whose label stayed is not treated as
queued: the next round's delivery takes the stale label off and goes ahead,
and the next enqueue labels the new head.

A repository that a person merges by hand is left out of
`MAIDAN_MERGE_QUEUE_LABELS` and is never enqueued. The full rules are in
[Result delivery](Result%20Delivery.md#the-change-flow-maidanchangeresult1--github_branch).

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
handed only threads whose footprint names that repository, so it never
claims work it would have to release. A thread without a footprint is never
handed to a worker that names a repository, so producers declare `repo` at
intake. The one exception is a review (`kind: review`): it reads and never
writes, so it needs no footprint, and its `repo` attribute is enough. The
review child Maidan spawns carries its change's repository there. Queue
depth's `repo` filter counts the same threads.

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
  is charged, so the worker's next `renew_claim` is refused (`409`
  `claim-lost`, MCP `-32012`), and it should stop;
- appends `ThreadStateChanged` and `ThreadCancelled` (`reason`,
  `cancelled_by`), and posts the reason on the thread as a message whose
  metadata is `{"notice": "cancelled"}`.

A second cancel answers `200` with the first one and `replayed: true`, and
changes nothing. A thread
that closed or was archived some other way answers `409`. A cancel publishes
no `ThreadReady` for the thread's dependents, but `claim_next` still counts a
cancelled dependency as finished, as it does any closed one.

## Closing a task that will not land

A task that nobody will finish, that duplicates another, or that a later task
replaces is closed as abandoned rather than cancelled, so the reason stays
typed:

| | REST | MCP |
|---|---|---|
| Close, as `duplicate`, `superseded` or `abandoned` | `POST /threads/{id}/close-abandoned` with `{"reason": "duplicate", "duplicate_of": "<id>"}` | `close_abandoned_thread` |

The thread must be open or in review. The call needs `thread:transition`
and access to the thread; with it the thread's producer, its claim holder or a
maintainer (`token:admin`) may close it. Anyone else gets `403`. The close is
audited as `thread.close_abandoned`, with the reason.
A thread that has a result keeps every landing gate; one with no result skips
the review gates, since there is nothing to review. `duplicate_of` must be a live
thread you can read in the same workspace. A `duplicate` must name it and
records a `duplicates` reference to it; a `superseded` close may name it and
records `duplicate_of supersedes` the thread; an `abandoned` close refuses it.

Nothing landed, so a dependency on the closed thread is not met. What happens
to its dependents depends on the reason:

- **`duplicate` or `superseded` with `duplicate_of`:** each open dependent now
  depends on `duplicate_of` instead, in the same transaction, and becomes
  ready when that thread lands. It gets a notice
  (`{"notice": "dependency_repointed"}`). The replacement's own dependency on
  the closed thread is dropped. When the move would make a cycle (the
  replacement already waits on that dependent), the dependent keeps its
  dependency and gets `{"notice": "dependency_repoint_refused"}` instead. The
  audit row lists `repointed_dependents` and `kept_dependents`.
- **`abandoned`, or `superseded` with no `duplicate_of`:** dependents keep
  waiting; re-plan the work or remove the dependency.

When the closed thread is a plan child, any dependent still waiting on it
after that is blocked with a notice, and the plan is told once.

## What ending early does to partial work

A task can stop before its work lands in three ways: a producer cancels it,
someone closes it as abandoned, duplicate or superseded, or a claimant that
restarted releases what its last run held. Each one changes Maidan's own
records in one transaction. A cancel or a close also queues, in that
transaction, the withdrawal of the pull request the change flow opened for
the task; the egress worker sends it to GitHub afterwards, never the cancel
itself. Maidan never ran the claimant's build, so the claimant cleans up its
own machine.

| Ending | Claim and fencing token | Thread | Branch and pull request | Queued deliveries | Result already written | The claimant's build box |
|---|---|---|---|---|---|---|
| `cancel_thread` (`POST /threads/{id}/cancel`) | Ended: assignee, lease and `claim_lease_id` cleared, worked time charged | `closed` around every gate, and parked unclaimable with the reason. A closed thread can only be archived, never reopened | The pull request the change flow opened for the task is taken out of the merge queue and closed, with a comment naming the reason, through the egress outbox. The branch is left | A change delivery not yet writing to GitHub is skipped (`thread_ended`). Messages are still sent, and the cancel's notice also goes to a linked Slack channel or GitHub issue, like any message | Kept and readable, but no longer listed among the channel's accepted decisions in the context pack | The claimant's to stop and clean up |
| `close_abandoned_thread` (`POST /threads/{id}/close-abandoned`) | Ended, as for a cancel | `closed`, and an explicit claim of it is refused (`409` `thread-ended`). The close parks nothing, and a park the thread already had stays. A thread with a result keeps its landing gates, so the close can be refused (`409` `conflict`) | As for a cancel, the comment naming the close reason (`abandoned`, `duplicate`, `superseded`) | A change delivery not yet writing to GitHub is skipped (`thread_ended`). Messages are still sent | Kept, and the reason the landing gates still apply. No longer listed as an accepted decision | The claimant's to stop and clean up |
| `release_own_claims` (`POST /workspaces/{id}/claims/release-own`) | Each live leased claim the member holds in the workspace is released, fenced by the lease read in the same transaction, worked time charged. A lapsed lease is left to the reaper, a lease-less assignment to its coordinator, and a claim on a thread the member can no longer read is kept until its lease lapses | Unchanged, `open` or `in_review`. An open one is back in the queue at once, and `claim_next` hands it out under its usual rules (blocks, required skills, dependencies, declines, order) | Left as they are | Still sent | Kept | What the old process left is the restarted claimant's to clean up before it claims again |

With [Linear claim comments](#claim-and-release-comments-on-linear) on, a
cancelled or abandoned task whose claim was said there also gets its
`RELEASED:` comment, but only when none of its pull requests merged (or a
handoff was recorded). A cancel does not undo a merge, so a task with a
merged pull request gets no `RELEASED:` comment. A release leaves the task
open and says nothing on Linear.

### How a claimant learns it should stop

A cancel or a close-abandoned notifies the member whose claim it ended,
unless that member did it itself, and that member's followers, as the
assignee's followers are told of any state change. The claimant finds out
in one of these ways:

- **Its next fenced call is refused.** `renew_claim` and `set_thread_result`
  with the old `claim_lease_id` answer `409` `claim-lost` (MCP `-32012`,
  `data.code` `claim_lost`) and store nothing. After a cancel or a close
  they answer `claim-lost`, not `thread-ended`, because the lease they name
  is gone. `acknowledge_claim` and `release_claim` with that lease answer
  the same `409` `claim-lost`.
- **A call the closed thread refuses.** Setting a footprint, requiring a
  skill, and marking the task's pull request ready or enqueueing it answer
  `409` `thread-ended` (MCP `-32013`, `data.code` `thread_ended`), and
  GitHub is not written. A message to a closed thread is still posted.
- **The notification.** A `thread_state_changed` notification on the
  task, for the member that held it.
- **The live events.** Both closes append `ThreadStateChanged`, whose
  `previous_assignee_id` names the member whose claim it ended (the
  `thread` it carries has no assignee any more); a cancel
  also appends `ThreadCancelled` with its `reason`, and posts the reason as a
  message with `{"notice": "cancelled"}`. A release appends
  `ThreadAssignmentChanged` for each thread, carrying its note. These kinds
  also fire for other threads and for ordinary work (a claim is a
  `ThreadAssignmentChanged` too), so stop only on an event whose `thread_id`
  is your task and that ends your claim: a `ThreadCancelled`, a
  `ThreadStateChanged` whose `to_state` is `closed` or `archived` (on a
  cancel, a close-abandoned, or a close by anyone else, its
  `previous_assignee_id` is you), or a
  `ThreadAssignmentChanged` whose `previous_assignee_id` is you and whose
  `assignee_id` is not.

On any of these, stop the build, then clean up the worktree, container or
branch checkout. Do it promptly: footprints and
[build slots](#build-slots) count live claims, so the moment the claim ends
Maidan hands that part of the repository and that slot to the next claimant,
even while the old build is still running.

### Who cleans up what

- **The claim:** Maidan, in the transaction that ends it.
- **The thread:** Maidan. A cancelled thread stays parked; clearing the park
  (`DELETE /threads/{id}/unclaimable`) does not reopen it.
- **The pull request:** Maidan, for the one the change flow opened for the
  task (the change-pull table records it, with at least one delivered round).
  The cancel or close queues its withdrawal on the egress outbox in its own
  transaction. The egress worker then reads the pull request once, runs the
  dequeue guards on that read (a `feature/agent-*` head in the repository,
  into an allowlisted base), removes the repository's merge-queue label if it
  is on, and, if the pull request is open, comments with the reason and
  closes it. A pull request a session opened and registered on the task, one
  only watched, one already seen closed, and one another task's result was
  delivered to since are left alone. Maidan also refuses to mark an ended
  task's pull request ready or to enqueue it.
- **The branch:** left. Deleting a branch is destructive, so it is the
  producer's or a person's call.
- **Queued deliveries:** Maidan. A change delivery (the branch commit and
  the draft pull request) whose task was cancelled or closed as abandoned,
  or is archived or deleted, is skipped with `thread_ended` if it has not
  started its GitHub writes when the cancel commits; one already writing
  can finish. Messages about the task (the cancel notice, the Slack reply)
  still go out.
- **Results:** they stay, as the record of what was produced. A cancelled
  or abandoned task's result is not listed among its channel's accepted
  decisions in the context pack: it was never accepted.
- **The build box:** the claimant, always.

Current gaps, read from the code and not yet decided:

- A change delivery already writing to GitHub when the cancel commits can
  finish, and can open its draft pull request after the withdrawal was
  queued. That pull request is not withdrawn.
- The withdrawal only reaches pull requests the change-pull table tracks
  under the task. One the change flow delivered to under another task's
  row is not found from this one.

Closed since: the withdrawal comment posts once on a pull request with up to
1,000 comments. It starts with a hidden marker naming the withdrawal, and a
retry after GitHub took the comment but failed the close finds it and only
closes. The retry reads at most 1,000 comments, so a busier pull request can
get a second one.

## Task attributes

A thread can carry attributes that say what kind of work it is, who or what
should do it, and whether it needs a threat model:

```json
{ "kind": "change", "engine": "unattended", "threat_model": false,
  "size": "M", "acceptance": "The bridge retries twice; its test proves it." }
```

`kind` is `change` (the default), `plan` or `review`. `engine` says which kind
of worker should take the task: `unattended` (an automated worker), `session` (an
interactive agent session) or `human` (a person). It is absent when nobody has
routed the work yet. Maidan uses the engine for routing, for the
[handoff](#handoff-at-non_converging) and for the per-engine metrics; what
runs behind each name is up to you.
`threat_model` marks work that needs one. `size` (`S`, `M` or `L`) is the
planner's estimate, and `acceptance` says how a reviewer verifies the task;
the worker sees it in the thread context. `linear_issue` names the Linear
issue the task is for and `repo` the repository it changes (`owner/name`); a
workspace can keep the pair unique among its open threads (see
[One owner per ticket](#one-owner-per-ticket)). The lane is not an attribute: it is
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
handed only work routed to it. A worker that passes `engine: "unattended"` is never
handed session or human work, and work nobody routed waits for a claim that
names no engine.

A workspace can route that work by default. Maidan creates threads that name
no engine: schedules fire them, inbound A2A messages open them, a message
becomes one, and a plan child or a REST or MCP create can leave the engine out.
`PUT /workspaces/{id}/default-engine` with `{ "engine": "unattended" }` (`token:admin`,
audited as `workspace.default_engine.set`) routes every thread created in the
workspace afterwards with no engine to `unattended`, in the transaction that creates
it, so a worker's `claim_next` with `engine: "unattended"` is handed it. An engine
named at creation still wins, threads created before keep their routing, and
`{ "engine": null }` turns the default off. `GET` on the same path reads it
(`workspace:read`).

A worked pass through the surface: intake files a review for the automated
worker and a plan for a session,

```http
POST /channels/{cid}/threads
{ "title": "review the claim queue change",
  "attributes": { "kind": "review", "engine": "unattended", "threat_model": true } }

POST /channels/{cid}/threads
{ "title": "plan the metering rollout",
  "attributes": { "kind": "plan", "engine": "session" } }
```

The automated worker's `claim_next` with `{ "engine": "unattended" }` is handed the
review; the plan waits for a session claim. If the review turns out to
need a person instead, `PUT /threads/{id}/attributes` with
`{ "kind": "review", "engine": "human", "threat_model": true }` reroutes
it whole, and the metrics read for the workspace
(`GET /workspaces/{id}/swarm-metrics?engine=unattended`) counts only the work
that was routed to `unattended`.

### A review's author family

A review must come from another model family than the change it reviews.
When a change Maidan tracks enters review, Maidan files the review child
itself and records the author's family. A producer filing a review of a
change written elsewhere (an outside pull request, a branch another model
wrote) names the author's family in the attributes:

```json
{ "kind": "review", "engine": "session", "author_family": "openai" }
```

`author_family` is `anthropic`, `openai`, `google` or `xai` (any case;
anything else is refused). The rule holds on every path a thread takes to a
member: `claim_next` skips the thread for a member whose usage reports name
that family, and a direct claim or an assign to one (REST and MCP) is
refused with a 409 naming the family. So an agent running an OpenAI model is
never handed a review of OpenAI-written work. One family is stored per change: a later
`PUT /threads/{id}/attributes` or `set_thread_attributes` that omits it
keeps it, and one that names another is refused with a 409, so nobody who
may set attributes, the change's author included, can lift or move it. A
member that has reported no usage in the workspace has no known family and
is not refused.

**Who may claim a review.** Any member whose usage in the workspace never
named the author's family. That is the whole rule: there is no one family a
review is meant for. The review child Maidan spawns carries `kind: review`,
`author_family`, and its change's `engine`, `repo` and `lane`, so a seat that
claims or reads queue depth filtered by engine and repository sees it where it
sees the change. It also carries a `review_family`, the first family Maidan
knows that is not the author's (`openai` for an Anthropic author). That is a
label and nothing routes on it: a seat of any other family, DeepSeek or
Google for an Anthropic change, may claim the review. **A review seat filters
on `kind=review` (with its engine and repository), never on
`review_family`:** a seat that also filters on its own family never picks a
review whose label names another, and those reviews wait for nobody.

A seat that knows its own family declines a review of that family's change
when it is handed one: `release_claim` with `decline: "own_family"` releases
the claim and tells `claim_next` not to hand that review to this member
again. The member's next `claim_next` gets the next eligible thread, or
nothing, instead of the same review; every other member is still handed it.
The decline is part of the release, so only the holder of the live lease can
record one, and only for itself; `release_own_claims` records none. It is
refused, and nothing is released, on a thread that is not a review naming
its `author_family`; a token that does not hold the claim is answered not
found, as a plain release is. It applies to `claim_next` only (a direct claim or assign
is the caller's choice), and needs no expiry: a review's author family is set
once, and the next review round is a new review thread. A seat no longer
needs to report a zero-cost usage record to stop being handed its own
family's reviews.

**The holder is judged again when it writes the review's result.** A
member's family comes only from its usage reports, and every claim-time check
reads them as they stood when the thread was handed out: a member with no
usage yet passes them all and is handed each own-family review once. So the
result write, the last step before a verdict is stored and delivered, checks
again, on two kinds of evidence:

- **The holder's usage.** Any usage record of the holder in the workspace
  that names the author's family, reported before the claim or during it.
- **The result itself.** A producer that runs its model outside Maidan's
  usage ledger says in the result which model served it: `served.family`,
  `served.provider` with `served.model`, or a `route[]` entry's `provider`
  with `model`
  ([Result Delivery](Result%20Delivery.md#what-maidan-reads-from-the-envelope)).
  These are named the way a usage record is, so `"provider": "Anthropic"`
  and `"model": "claude-opus-4-5"` both name `anthropic`.

If either names the author's family, the write is refused with a 409 (MCP
invalid params) naming the family. Nothing is stored, so nothing is
delivered and no review decision is applied. The usage report itself is
accepted, so the spend stays on the ledger and the budget, and the claim
stays held: the seat releases it with `decline: "own_family"`. Only the
thread's own workspace is read, and a caller that does not hold the claim
is answered by the claim fence (`claim_lost`) before anything about the
review's family. A change, or a review that names no `author_family`, takes
its result under the claim fence alone.

Reporting usage before writing the result is recommended, not required:
then a seat that turns out to be of the author's family is stopped before
its verdict is posted.

What the rule does not cover:

- **A result with no evidence is stored.** A holder with no usage naming any
  family, whose result declares no `served` or `route`, has nothing to judge
  it by, and its verdict is stored and delivered. A seat that reports its
  usage after the result is judged on its usage only from then on: that
  usage refuses it every later review of the family, but not the verdict
  already delivered.
- **A family is what the evidence says it is.** A seat that reports, or
  declares, a model of another family than the one it ran, or one that
  names no family Maidan knows, is judged on what it said.
- **Workspace import carries no family to check.** An import restores a
  workspace's members, channels, threads (with their holders) and
  conversation, but no thread attributes, usage records or results. An
  imported review names no `author_family` until someone sets one (refused
  while its holder's usage names that family), and every result written
  afterwards is judged as above. Import checks only that a holder belongs to
  the imported workspace.

### One owner per ticket

A ticket should have one owner at a time in each repository. When the router
stamps a thread with its Linear issue, nothing else stops a second path from
opening another thread on the same work: one ticket can get a hosted agent's
draft and a laptop session's pull request at once. A ticket often has deliberate halves in
two repositories (a pull request in a web app and one in its API), so the rule that stops
two owners is per ticket **and** repository, and a workspace makes it a
refusal.

**The router stamps both attributes.** Every thread the router opens for a
ticket carries `linear_issue` and `repo`:

```json
{ "kind": "change", "engine": "unattended", "linear_issue": "ENG-5150", "repo": "example/widgets" }
```

`repo` is `owner/name` (a GitHub URL is read as its `owner/name`, and any case
is folded). A thread missing either attribute is not held to the rule, so a
router that leaves out `repo` gets no protection. Find-by-attribute filters on
it too: `GET /workspaces/{id}/threads?attr.repo=example/widgets`, or MCP
`find_threads` with `attr.repo`.

**The setting.** `PUT /workspaces/{id}/unique-thread-attributes` with
`{"unique": [ ["linear_issue", "repo"] ]}` (`token:admin`, audited as
`workspace.unique_thread_attributes.set`) lists key sets. Each set is unique as
a tuple among the workspace's **open** threads: no two open threads that both
give every key of the set may give the same values for all of them. A thread
that lacks any key of a set is not checked against that set. Only threads of
`kind: change` are held to the sets: a change is the work that owns a ticket.
A review or plan thread on the same ticket and repository never conflicts and
never holds a tuple. That covers a red-team review, which carries the ticket
and the repository of the change it reviews. A single-key set
works too: `[["linear_issue"]]` is one open thread per ticket whatever the
repository. `{"unique": []}`, the default, keeps nothing unique, an empty set
is refused, and `GET` reads the setting (`workspace:read`). Today the keys are
`linear_issue` and `repo`. The other attributes are kinds and routing, where
"only one open thread" would be a WIP limit, not ownership.

With a set in force, a thread write that would give a second open thread the
same tuple is refused and writes nothing:

- **Where.** Creating a thread (REST `POST /channels/{cid}/threads`, MCP
  `create_thread`, and every other create path that stores attributes, plan
  children included), and setting attributes on an open thread
  (`PUT /threads/{id}/attributes`, MCP `set_thread_attributes`).
- **The answer.** REST answers `409` with type
  `https://maidan.world/problems/conflict`. Its `key_set` is the set the tuple
  was held under (`["linear_issue", "repo"]`), and its `holder_thread_id` is
  the open thread holding it. MCP answers error `-32011`, whose `data` carries
  `status` `409`, the same `type`, `key_set` and `holder_thread_id`. The
  holder is named only when the caller may read it. A caller outside its
  private channel or DM learns that the tuple is taken, not by which thread.
  Work in the holder, or close it first.
- **What holds.** An open thread is one that is not closed, archived or
  erased. Only open threads in the same workspace hold a tuple:
  - closing the holder frees the tuple, so the ticket can be reworked;
  - another workspace's threads never count;
  - the holder may restate its own attributes;
  - a closed thread can be given any attributes, since it holds nothing.
- **Races.** Two writers of one tuple cannot both win. On Postgres each takes
  a transaction-scoped advisory lock on (workspace, key set, values) before
  looking for a holder. The second writer waits for the first to commit, then
  sees its thread. Writers that check several sets take the locks in one
  sorted order. On SQLite the check runs inside a transaction that already
  holds the database's one write lock. A unique index cannot express the
  rule, because whether a tuple is held depends on the workspace's setting
  and on the other thread's state.

What it does not do:

- Turning it on does not close duplicates already open. The next write on
  such a tuple is refused.
- Reopening a closed thread is not checked.
- A planner that stamps the parent's ticket and repository on each plan child
  is refused while the parent is open. Put them on the parent only, or give
  each child its own repository.

### Claim and release comments on Linear

A task's attributes may name the Linear issue it is for
(`"linear_issue": "ENG-1234"`; refused unless it is an issue key). A
workspace can then say each task's claim and release on that issue. The
switch is **off by default**: `PUT /workspaces/{id}/linear-claim-comments`
with `{"enabled": true}` (`token:admin`, audited) turns it on, and `GET`
reads it (`workspace:read`). While it is off, nothing is said on Linear.

With it on, the change sweep says each of these once per task:

- `CLAIM: <engine>/<task id> claimed in Maidan by <handle>: <title>` when
  the task is first claimed;
- `RELEASED: <engine>/<task id> ended without merging: <title>` when, after a
  claim was said, the task closes or is archived with no merged pull
  request.

`<engine>` is the task's engine, or `maidan` when it has none. The comments go
out through the Linear delivery path with the same key and the same
per-team allowlist as a result's comment. There is no assignee and no new
identity. The issue is looked up first, and a comment is queued only when its
team is blessed for the workspace. An issue in another team, or one that
cannot be found, is recorded as skipped and never written to. The worker
checks the allowlist and the team again when it posts. A notice is recorded
in the same transaction that queues it, so it is said once even with
several replicas sweeping.

### Handoff at `non_converging`

A task routed to the automated engine (`unattended`) that stops converging is handed
to a session, once. The change sweep flags a task `non_converging` when it is
still open after the round limit (3), or when its diff grew in each of the
last three rounds. For a task routed to `unattended`, it then:

- ends the claim, charging its worked time as any claim end does, so the
  worker's next renew is refused;
- sets the task's engine to `session`, so an `unattended` worker's `claim_next` never
  takes it again, and leaves the task open for a session;
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

The watch's thread ends with its pull request. When the change sweep sees
the pull request merged, or closed without merging, it closes the thread
straight from `open` or `in_review`, as the watcher, with one message naming
the pull request and how it ended (`owner/name#N merged, so this watch is
closed.`, or `closed without merging`), and the `ThreadStateChanged` event
tells its watchers. A merge also records `ThreadLanded`; a close without one
records that nothing landed, so a thread depending on the watch keeps
waiting. No land or review gate holds the close: a thread that only watches
has no result or delivery of its own for them to judge. A thread that also
carries a result, an armed delivery, or a pull request the change flow
delivered or a session registered is a change, and settles as one below.

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

A token that may not read the checks through GraphQL (a fine-grained token
never can) reads them from the Actions and commit-status APIs instead: a
REST call for the head's workflow runs, one for each run's jobs and one for
the commit statuses, for each pull request. Polled every tick for every open
pull request, that spent the token's hourly budget. So with the GitHub
ingress configured (`MAIDAN_GITHUB_WEBHOOK_SECRET`, and GitHub's
`workflow_run`, `check_suite`, `check_run` and `status` events delivered to
`POST /integrations/github/events`), CI notices are event-driven: the
ingress marks the tracked pull requests an event names (by number, or by the
head their CI was last read at) and makes no GitHub call, and a tick reads a
pull request's CI only when an event arrived since its last read, its head
moved, or that read is older than the backstop,
`MAIDAN_CHANGE_CI_BACKSTOP_SECS` (1800 s by default), which covers a dropped
delivery. Every tick still makes the one GraphQL read, so closes, merges and
the ready label are seen as before. Without the ingress no event arrives, and
every tick reads CI as it always did. `maidan_github_events_total{event,
outcome}` counts each delivery (`marked`, `unmatched`, `handled`, `ignored`,
or `refused` for a bad signature), so a forwarder's sent count can be checked
against what Maidan took in.

The same read says what else holds the pull request back, and `ci` records
it: GitHub's `merge_state`, the unresolved review threads (a review bot's
count too: `unresolved_threads`, and links to the first 20 with who opened
them), and `missing_checks`, the checks the base branch's protection requires
that never reported though every check that did has finished. Together they
make `gate_reasons`: one sentence each for unresolved threads, a merge state
of `BEHIND`, `BLOCKED`, `DIRTY` or `DRAFT`, and each missing check. When the
reasons change and two reads in a row agree on the change, the thread gets
**`gate_blocked`** with the new `reasons`, or **`gate_clear`** when none are
left. GitHub's merge state flickers while checks and the merge queue settle,
and a reason that holds for only one read (one change-sweep tick) says
nothing; a real change arrives one tick later.
Reading the protection needs a credential that may read the repository's
administration settings; without it no check counts as missing.

`ci.merge_queue` follows the pull request through the base branch's merge
queue: `ready_to_merge` (it carries the `ready-to-merge` label and is not
queued), `queued`, `dequeued` with GitHub's `reason` (`MERGE_CONFLICT`,
`FAILED_CHECKS`, ...), and `landed` once its merge is recorded. Entering the
queue is said as **`merge_queued`**, and leaving it without merging as
**`merge_dequeued`** with the reason in `reasons`. A landing is the change
sweep's usual one: the thread closes with `ThreadLanded`.

Each review submitted on the pull request, a bot's included, is said once as
**`review_submitted`**, naming the `reviewer`, the `review_state`
(`approved`, `changes_requested` or `commented`) and the `head_sha` the
review was on, which can be an earlier head than the current one. It comes
from the same read, the newest 20 reviews; a draft or a dismissed review
says nothing.

A pull request closed without merging counts as landed only when its head
had commits its base lacked while it was open and is on the base branch now,
as when a stack tool lands the commits itself and closes the pull request.
The sweep asks GitHub once, when it first sees the close. A branch reset to
its base and closed was never ahead, so it lands nothing. Any other close is
a close without a merge, said once and leaving the task open (a watch
closes, as above).

A merged change lands through its thread's gates: separation of duties, the
review requirement and the land gate. When one refuses, the thread stays
open and gets one message saying why (`#N merged, but this thread stays open
until its gates pass, ...`). The merge is not settled, though: each sweep
tries the landing again, in one store transaction and with no GitHub call,
and the first sweep after the gate lifts (a reviewer approves, the land gate
records a qualifying pass) closes the thread with `ThreadLanded`, so its
dependents become claimable without anyone closing it by hand. The refusal
is said once however many sweeps try again. A thread closed or cancelled in
the meantime is never landed, and its merge settles. Each sweep settles at
most 25 closes a workspace, those never tried first and then the refusals
tried longest ago, so refusals waiting on a person hold back neither a new
close nor one another.

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

## Measuring the swarm

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
task was routed to (`by_engine`: `unattended`, `session`, `human`, then unrouted),
by size (`by_size`: `S`, `M`, `L`, then unsized) and by risk tier
(`by_risk`: standard work, then `threat_model` work). A split leaves out a
slice with no tasks. If you route work to engines by size or risk, compare
engines within one size or one tier: across the whole repository the
comparison mostly measures the routing. Cost per merged change in each engine
slice compares the engines on cost.

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
named), so a trial on a few issues can be read apart from other work in the same
repository. Use these numbers to decide when to add workers.

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
