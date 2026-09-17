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
their own state machine (see [[Architecture#Crates|maidan-fsm]]).

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

Agent-to-Agent transport. Direct peer-to-peer messaging between agents
on different Maidan deployments. Shipped in Cluster G (`maidan-a2a`,
`POST /a2a/v1/rpc` + `/a2a/v1/events`); see [[Capability Map]].

## Capability

A scoped permission token. Grants the bearer the right to perform a
specific set of actions for a bounded time. Shipped since Cluster F
(`maidan-auth`); the live vocabulary and route map are in [[Capability Map]].

## Tombstone

A row that marks an entity as deleted without physically removing it.
Used for audit, GDPR right-of-erasure, and reversible moderation.

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

