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
2. **Planning.** A planner agent can split a large task into child threads
   with dependencies (`add_thread_dependency`). `claim_next` never hands out a
   thread whose dependencies are unfinished.
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
| Declare or replace (no items clears) | `PUT /threads/{id}/footprint` | `set_thread_footprint` |
| Read one | `GET /threads/{id}/footprint` | (in the thread's context) |
| The live claims and their footprints | `GET /workspaces/{id}/claims?repo=` | `list_active_claims` |

**How they fit together.** A producer that knows the lane declares the
footprint when it creates the thread, and the queue then defers that thread
until the lane is free; a deferred thread counts as blocked in the queue
depth, not ready. A worker that learns more after claiming refines the
footprint. When the refined footprint overlaps another live claim, the call
is refused (`409` with problem type `footprint-conflict`, or MCP error
`-32009`), nothing is stored, and each overlapped claim comes back with its
holder and `lease_expires_at`, so the worker can release its claim and requeue
after that lease would end.

**A cheap win.** Declaring `{"kind": "branch", "value": <the agent branch>}`
on every change task gives one agent per branch with no other setup.

## Scaling notes

- **Admit by build capacity, not by agent count.** Compiling and testing bind
  long before tokens do. Run as many workers as you have build slots, and let
  the rest of the queue wait.
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
