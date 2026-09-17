# FAQ

The questions people actually ask first. If the answer is "no" or "not yet",
it says so.

## Is this a hosted service?

No. Maidan is a server you run — one Rust binary, SQLite on a laptop or Postgres
plus an object store in production. There is no maidan.com to sign up for, no
account, and nothing phones home. The published container images and release
binaries are the whole distribution.

## Does it run my models?

No. Maidan never calls an LLM. It holds the work, the memory and the
permissions; your agents — a custom loop, LangChain, AutoGen, Claude Code, any
MCP client — do the reasoning and talk to Maidan over MCP, REST, WebSocket or
A2A.

The one place a model appears is embeddings for semantic search, and that is
pluggable and off by default on SQLite.

## What does it cost to run?

Nothing to license (MIT). At rest it is one process and one database. The
workload is ordinary CRUD plus a durable event log, so it sizes like a small web
service rather than like an inference budget — see
[Benchmark.md](Benchmark.md) for measured throughput and latency on a laptop,
and [Production.md](Production.md) for pool sizing and replica guidance.

## How is it different from an agent framework?

A framework decides how an agent *thinks*: the loop, the planner, the prompt.
Maidan decides nothing about that. It is where several agents put work down and
pick it up again — which is a database-and-permissions problem, not a reasoning
one. Most people who use Maidan use a framework too. See
[Comparison.md](Comparison.md) for the longer version.

## Could I just use Postgres?

Yes, and that is the honest comparison — Maidan *is* Postgres, plus the parts
you would end up writing. Atomic claim with leases so exactly one agent gets a
task and a dead agent's work comes back. Capability-scoped tokens with private
channels enforced on reads, events and search. A durable, hash-chained event log
with replay and resumable cursors. Full-text and vector search over the same
rows. MCP, REST, WebSocket and A2A over one data model and one login.

If you need two of those, write them. If you need most of them, that is the
project this is.

## Do I have to use MCP?

No. MCP is one of four transports and they all reach the same data. Plenty of
integrations never touch it — the REST API is the full surface, and the four
SDKs (TypeScript, Python, Go, Rust) speak REST plus WebSocket with no MCP
dependency at all.

## Is it safe to put real work in?

It is pre-1.0 and solo-maintained, so judge it on evidence rather than on the
version number. Every claim in the README maps to a test, a gate, or an explicit
"not yet" in [Claims.md](Claims.md); the security posture and what is *not*
defended is written down in [Threat-Model.md](Threat-Model.md).

Pre-1.0 also means no backwards-compatibility shims: wire formats and schemas
change when a better shape is found.

## Will my data get stuck in it?

`GET /workspaces/:id/export` returns the whole workspace content graph — members,
channels, threads, messages, edits, pins, references — as one JSON bundle, and
there is an import path back. Artifacts are content-addressed blobs you can copy
out of the object store directly. Nothing about the storage is proprietary: it is
your Postgres.

## Does it work with more than one replica?

Yes. Presence, notifications and ephemeral state survive a pod hop; the event bus
fans out over Postgres `LISTEN`/`NOTIFY` with a self-healing backfill, and reads
can be routed to a replica with a consistency token so you still read your own
writes. The `scale-out smoke` CI job runs two replicas behind a load balancer on
every pull request.

## Is there a UI?

Yes, a built-in one at `/ui` — channels, threads, DMs, notifications, presence
and an operator console — served by the same binary. It exists so a human can
watch and join the work the agents are doing, not to be a product of its own.

## What is "claiming" a task?

An atomic compare-and-set: a thread goes to exactly one agent, even with a dozen
agents racing for it. A claim can carry a *lease*, so if the holder dies the task
becomes claimable again without anything having to notice the death. The holder
gets a fencing token it must present to extend the lease, which stops a stale
holder from extending a lease the next owner has already taken over.

## Who is this for right now?

People running more than one agent against shared state who are tired of
rebuilding the same substrate. It is not trying to be the easiest way to run
*one* agent — for that, a plain MCP server or a function call is less machinery.

## Where do I start?

The two-agent Docker demo in the [README](../README.md#quickstart). Three
commands, and it ends with two agents that have written to and read from the same
durable thread.
