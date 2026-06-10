# Cluster 106.0 — Bulk store reads

**Theme:** Kill the N+1 query patterns in context assembly with batched store reads.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XX · tag **`v106.0.0`**.

**Predecessor:** [[Clusters/Cluster 101.0]]; context export from [[Clusters/Product Ladder 59+]] / [[Clusters/Cluster 82.0]].

---

## Problem

The context builders issue **one query per row**. In `crates/maidan-server/src/thread_context.rs`:

- line **149** — `store.list_threads(channel.id)` is called **per channel** when assembling workspace context (the `Store` trait only offers `list_threads(channel_id)`, `store.rs:93` — no workspace-level bulk read).
- line **91** — `store.list_references_from(RefSide::Message, message.id.0)` is called **per message**.
- line **117** — `store.list_message_edits(message.id, 20)` is called **per message**.

A 100-message thread → ~200 extra round-trips; a 50-channel workspace context → ~50 thread queries. Latency scales with content size, and the cost multiplies under the multi-replica load this ladder enables. This is the highest-ROI, lowest-risk change in the phase.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Store** | Batched reads: `list_threads_for_workspace(workspace_id)`, `list_references_from_many(side, ids)`, `list_message_edits_for_messages(ids, limit_per)` (or equivalent `= ANY($1)` / `IN (…)` queries). Postgres **and** SQLite, with dialect-parity tests. |
| **Server** | Rewrite thread + workspace context assembly to use the bulk methods — **O(1) round-trips per context**, not O(messages)/O(channels). |
| **Tests** | A query-count regression test (counting store wrapper or sqlx logging) asserting context endpoints issue a bounded number of queries; existing correctness e2e unchanged. |
| **Docs** | [[Query-Tuning]] note + a [[Decisions]] ADR ("bulk reads for context assembly; store grows batched accessors as call sites need them"). |

## Non-goals

- A caching layer (correctness-first; caching is a later, separate concern).
- Changing the context response shape or ordering contracts (that was [[Clusters/Cluster 82.0]] pagination).
- Generic query-builder abstraction over the store — add concrete batched methods only.

## PR ladder (suggested)

| # | Title |
|---|--------|
| 106.0.1 | `feat(store): batched thread/reference/edit reads (pg + sqlite)` |
| 106.0.2 | `refactor(server): assemble thread + workspace context via bulk reads` |
| 106.0.3 | `test(server): context_query_count_regression` |
| 106.0.4 | `docs(query-tuning): bulk context reads + Decisions ADR` |
| 106.0.retro | `docs(retro): Cluster 106.0 + v106.0.0 tag prep` |

## Exit criteria

- Context endpoints issue a **bounded** number of queries independent of message/channel count (asserted in a regression test).
- New bulk store methods pass the shared parity suite on both backends.
- No change to context response content or ordering.
- `v106.0.0` tagged after retro.

## Ordering & risks

- **Do first** in Phase XX — independent, highest ROI, unblocks honest perf claims for the rest of the ladder.
- **Risk — SQLite `= ANY` unsupported:** use expanded `IN (?, ?, …)` with a capped batch size (chunk large id sets) rather than Postgres array binding; keep ordering deterministic.
- **Risk — over-fetch:** batched edit reads must preserve the per-message limit (e.g. 20) — fetch with a windowed query, not all edits for all messages unbounded.

## References

- [[Clusters/Product Ladder 102+]] Phase XX
- [[Clusters/Product Ladder 59+]], [[Clusters/Cluster 82.0]] (context baseline)
- [[Query-Tuning]], [[Decisions]], [[Architecture]]
