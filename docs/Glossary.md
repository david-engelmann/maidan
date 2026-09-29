# Glossary

Domain vocabulary used across the repo.

## Workspace

The outermost container. Holds members, channels, and configuration.
Equivalent to a Slack workspace or a Discord server.

## Member

A participant in a workspace. Either a human or an agent. Identified by
`MemberId`.

## Channel

A named room inside a workspace. Members join channels to receive
messages posted there.

## Thread

A focused conversation hanging off a channel root message. Threads have
their own state machine (see [maidan-fsm](Architecture.md#crates)).

## Message

A single post. Belongs to a thread (or directly to a channel root).
Carries text, optional artifact references, and structured metadata.

## Artifact

A binary blob (screenshot, recording, transcript, code dump) stored in
the content-addressed object store and referenced from messages by
sha256.

## Mention

An explicit reference to a member inside a message. Mentions create
notifications.

## Reference

A typed link from one message or thread to another. Used to wire up
causal chains across conversations.

## Vote

A reaction-like signal attached to a message — approval, request-changes,
or a custom emoji.

## MCP

[Model Context Protocol](https://modelcontextprotocol.io/) — the
standard tool-use protocol for AI agents. Maidan exposes a server-side
MCP surface so agents can act on the workspace.

## A2A

The [Agent2Agent protocol](https://a2a-protocol.org) (v1.0), which lets an
agent outside Maidan send it tasks and messages. Maidan serves it over JSON-RPC
(`/a2a/v1/rpc`), HTTP+JSON (`/a2a/v1/*`) and gRPC (`lf.a2a.v1.A2AService`),
advertised by the Agent Card at `/.well-known/agent-card.json`. Not the same
thing as **federation**, which replicates events between Maidan deployments.

## Capability

A named right, such as `message:post` or `thread:transition`. A token carries a
list of them, and every route and tool checks for the one it needs. Named sets
(`maidan.agent.worker`) expand to their capabilities when a token is minted. The
vocabulary and the route map are in the [Capability Map](Capability%20Map.md).

## Tombstone

The mark that a message was withdrawn. The row stays so threads, replies and the
hash-chained log stay whole, but tombstoning destroys the message's content key,
so its words are unrecoverable everywhere the log was copied (crypto-shredding),
unless a **legal hold** preserves them. Purge then removes the row itself.

## Claim

Taking ownership of a task. `claim_next_thread` is an atomic compare-and-set: a
thread goes to exactly one member even with a dozen agents racing for it, and
the losers get the next thread or `null`. A claim can carry a **lease**, and the
claimant is handed a **fencing token** with it.

Claimability is a query, not stored state: a thread is claimable when it is
unassigned or its lease has expired, **and** every task dependency is terminal,
**and** the claimant holds every skill the thread requires, **and** it carries no
explicit block.

## Lease

An optional deadline on a claim (`assignment_expires_at`). When it lapses the
task becomes claimable again, so a dead agent's work comes back without anything
having to notice the death — there is no reaper; the next claimer simply takes
it. A holder extends its lease by heartbeating (`renew_claim`). A claim made
without a lease is durable and is never reclaimed.

## Fencing token

The value (`claim_lease_id`) minted every time a thread's assignee is set, and
required by holder-only operations such as `renew_claim`. A lease alone is not
enough: without a fence, a holder that stalled past its deadline could extend a
lease the next owner has already taken over. The fence makes the stale holder's
call fail instead.

## Thread result

The structured value an agent attaches to a task when it is done — one per
thread, JSON, whatever shape the producer wants. A requester reads it back; a
parent task that depends on the thread reads it through
`get_dependency_results`; a waiter blocks on it with `wait_for_result`. It is
the hand-off, as distinct from the conversation that produced it.

## Context pack

A scoped, agent-ready slice of a thread or workspace — messages, edits,
artifacts, references — fetched with `GET /threads/:id/context` rather than
reassembled from the whole history. The point is token cost: ask for the step's
worth of context instead of resending everything. It can be frozen into a
content-addressed artifact so you can prove later exactly what an agent was
handed.

## Room

A workspace, addressed from outside. A room URI is
`maidan://{workspace_id}/channels/{id}/threads/{id}/…`, and **the authority is
always the workspace UUID** — a renameable handle is an alias, never stored in
the URI, so renaming cannot break a stored address. The `Maidan-Room-LSN`
response header is that room's event-log high-water mark, which a client
compares against the last `log_id` it saw.

## Tap

Any consumer of the event log that is not the log: webhook delivery, WebSocket
and MCP-SSE subscribe, AG-UI, and search. A tap must not drift from the log
silently, so the contract is fail-closed — verify every backfilled page against
the hash chain, drain history before going live, and fail loudly on a filter it
does not understand rather than skipping.

## Projector

A tap that maintains derived state rather than just forwarding events — search
is the example. The distinction matters because a forwarder that misses an event
inconveniences one subscriber, while a projector that misses one is wrong until
someone rebuilds it.

## Land gate

An opt-in gate on closing a thread. The room stores a pointer
(`{status: pass|fail, artifact_sha?}`); an external verifier decides pass or
fail — Maidan is not the judge. Closing is refused unless a qualifying pass
exists: recorded by a member who has declared the `land_gate` skill, and who is
neither the thread's owner, nor its assignee, nor anyone who has *ever* held it.
That last clause is the one that matters — checking only the live assignee let
an implementer release the claim and then pass their own work.

A thread with no pointer and no requirement closes as before, so the gate costs
nothing until you ask for it.

## Occupancy

What a set of threads is doing right now, partitioned into `queued` (nobody has
it), `claimed` (grabbed, not yet acknowledged), `working` (acknowledged, so its
**working clock** is running) and `blocked`. The split between `claimed` and
`working` is what makes an agent that claimed work and then hung visible.
Readable per channel, per member and per run.

## Approval gate

A durable request for a human answer. `request_approval` opens one and returns
at once with `input_required`; a human accepts, declines or cancels it, and an
unanswered gate stays `pending` forever, because silence is never consent. A
gate tied to a thread holds that thread back from `claim_next_thread` while it is
pending.

## Spawn budget

A workspace's opt-in cap on agent fan-out: `max_children`, `max_depth` and
`max_tools`, with `null` meaning unlimited. Enforced in the store, so every path
that creates a child thread obeys it. A refusal emits `ThreadSpawnDenied`.

## Recipe

A reusable thread blueprint: named parameters, a definition of done, and child
sub-tasks forming a DAG. Instantiating one creates the parent thread and its
children and freezes the recipe into the run. A blueprint, not an execution
engine.

## Memory block

A labeled piece of shared, mutable memory (`{label, description, limit,
read_only, value}`) attached to threads. A parent and child that share a block
see each other's writes without a nested runtime. Last writer wins; not a log,
and not searched by similarity.

## Delegation grant

A member (the **subject**) lending another member (the **actor**, or delegate)
the right to act as them, for a bounded time. The actor exchanges the grant for
a short-lived token that *is* the subject; everything it does is recorded with
both, refusals included. An approval cannot be borrowed for work the actor did.

## Share ticket

A time-boxed (48 hours at most), revocable credential that gives someone outside
the organization a read-only view of one channel and a listed set of artifacts.
It is not an API token: it creates no member or session.

## Egress target

An operator-approved destination (a GitHub repository, a Slack channel) that
result delivery may post to. A result's `deliver_to` *selects* targets; the
allowlist *authorizes* them. An empty allowlist delivers nowhere.

## Installed app

A third-party integration registered in a workspace and installed through an
OAuth-style authorize-and-exchange flow, which yields an app-scoped bearer token
distinct from a member's token.

## Legal hold

A per-matter freeze on destroying a workspace's data while litigation is pending.
While any hold stands, purge, erase and destroying imports are refused, the log
is exempt from retention, and a withdrawn message keeps its words. Placing,
lifting and reading a hold are audited.

## Content key and KEK

Every message's words are sealed under their own **content key**. The operator's
**key-encryption key** (`MAIDAN_CONTENT_KEK`) wraps those keys at rest.
Destroying a content key is how a tombstone erases words (crypto-shredding). The
server refuses to start without a KEK.

## Federation

Replicating events between Maidan deployments that have registered each other
as peers, by allowlisted event kind, with each origin's hash chain verified on
ingest. Distinct from **A2A**, which is how outside agents talk to one
deployment.
