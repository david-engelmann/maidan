# Cluster 106.0 retro — Bulk store reads

> Tag **`v106.0.0`**. First cluster of Phase XX (hot-path hardening).

## What shipped

- **Batched store accessors** — `list_threads_for_workspace`, `list_references_from_many`, `list_message_edits_for_messages` on the `Store` trait, both backends. Postgres binds id arrays (`= ANY($1)`); SQLite expands chunked `IN (?, …)` (400/query). Edits use a windowed `ROW_NUMBER() OVER (PARTITION BY message_id …)` to keep the per-message cap. `bulk_reads` parity test. (106.0.1, #293)
- **Context rewrite** — `thread_context.rs` assembles thread + workspace context via the batched reads instead of per-row loops, eliminating the three N+1s, with response content + ordering preserved byte-for-byte (edits re-sorted into message-iteration order). (106.0.2, #294)
- **Query-count regression** — `context_query_count_e2e` counts `sqlx::query` tracing events and asserts a 40-message thread issues the same query count as a 3-message one; a sanity floor prevents a vacuous green. (106.0.3, #295)
- **Docs** — `docs/Query-Tuning.md` "Context assembly (bulk reads)" + a `docs/Decisions.md` ADR ("bulk reads for context assembly; the store grows batched accessors as call sites need them"). (106.0.4, this PR)

## What was deferred / not covered

- **Per-thread sub-context in workspace context** stays O(threads) — each thread is its own bounded context; the cluster bounds *per-context* queries, not the number of contexts.
- **Artifact-metadata reads** remain per-distinct-sha (`get_artifact_by_sha` in a loop). Out of the stated scope (the plan enumerated threads/references/edits), and usually near-zero since most messages carry no artifact sha. Flagged in the Query-Tuning note as a future batch candidate.
- A caching layer and a generic loader abstraction — explicit non-goals (correctness-first; concrete accessors over indirection).

## Surprises

- **Counting queries cheaply.** The `Store` trait has 137 methods, so a delegating "counting wrapper" was impractical and fragile. Counting `sqlx::query` `tracing` events instead is a light, backend-agnostic seam — but it needed a global subscriber (so the test lives in its own single-test binary) and a sanity floor (`>= 5` queries) so the equality assert can't pass vacuously if sqlx ever stops emitting events.
- **Edit ordering.** The batched edit read returns rows ordered by `message_id` (UUID order), but the response groups edits in message-*iteration* order. The rewrite re-sorts by message position so the output is unchanged — caught only because the existing `thread_context_e2e` asserts content.

## Decisions

- **Concrete batched accessors, added as call sites need them** — keep the store's runtime-checked-SQL model and honest per-method cost, rather than a query-builder/DataLoader abstraction or a cache. See `docs/Decisions.md`.

## Capability table extension

| Capability | Where |
|------------|-------|
| O(1)-query context assembly (no per-row N+1) | `thread_context.rs`, `Store::{list_threads_for_workspace, list_references_from_many, list_message_edits_for_messages}` |
| Query-count regression guard | `context_query_count_e2e` |

## Risks

- The query-count test depends on sqlx emitting `sqlx::query` tracing events; the sanity floor turns a silent instrumentation break into a failure rather than a false green.
- SQLite `IN (?, …)` chunking caps at 400 ids/query — far under the variable limit; very large id sets issue multiple statements (still bounded by chunk count, not row count).

## Next

Cluster **107** — pool & timeouts configurable (env-driven `max_connections`, `acquire_timeout`, statement timeout).
