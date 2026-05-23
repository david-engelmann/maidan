# Cluster C retro — Search + indexing

> Closing wave for Cluster C · target tag `v0.2.0`.

Cluster B made the workspace reachable. Cluster C makes it searchable.
Four delivery PRs (five with this retro) landed lexical search on both
backends, vector search on Postgres, and the async indexer pipeline
that future clusters will use for embedding generation.

## What shipped

- **PR #31** — `feat(maidan-search): full-text search (postgres
  tsvector + sqlite fts5)` — schema 0002, `Search` trait, both
  backends. Generated `tsvector` column + GIN index on Postgres;
  FTS5 virtual table + sync triggers on SQLite. Shared `tests/common`
  seeds the same corpus on each dialect; parity test asserts identical
  hit body sets.
- **PR #32** — `feat(maidan-server): http /search + mcp
  search_messages tool` — `GET /workspaces/:wid/search` returns
  ranked hits with `<mark>`-wrapped snippets; the MCP tool drives the
  same `Arc<dyn Search>` so the surfaces stay byte-identical.
- **PR #33** — `feat(maidan-search): pgvector embeddings + semantic
  search` — schema 0003 with `vector(1024)` + HNSW cosine index;
  `Search::upsert_embedding` and `semantic_search` round-trip via
  `pgvector::Vector`. SQLite returns `Unsupported`.
- **PR #34** — `feat(maidan-search): bus-driven background indexer`
  — `EventHandler` trait + `Indexer::spawn` that subscribes to
  `MessagePosted` / `MessageTombstoned` and reconnects with
  exponential backoff. Server wires `LoggingHandler` on boot;
  future clusters swap in real embedding generators.

## What was deferred

| To             | What                                                       | Why                                                                |
|----------------|------------------------------------------------------------|--------------------------------------------------------------------|
| Cluster D      | Real embedding generation (model loading + ingest)         | Needs the FSM cluster's thread-lifecycle hooks.                    |
| Cluster D      | Persistent event log / at-least-once delivery              | Indexer is at-most-once until the log lands.                       |
| Cluster D      | Per-model embedding tables / dimension variations          | Wait for multi-model use case.                                     |
| Cluster T      | Indexer lag metric on `/health`                            | Observability cluster.                                             |
| Cluster T      | `websearch_to_tsquery` Google-style operators in `q`       | Tuning.                                                            |
| Cluster T      | Score normalization across dialects                        | Postgres `ts_rank_cd` and SQLite `bm25` aren't on the same scale.  |
| Cluster F+     | SQLite vector support (`sqlite-vec`)                       | Extension is young; sqlx integration is immature.                  |
| Cluster H      | Faceted search (author / channel / kind filters)           | UI cluster.                                                        |
| Cluster X      | Release matrix fix (macOS x86_64 build dropped)            | `v0.1.0` release didn't auto-create; cleanup PR before v0.2.0.     |

## Surprises

- **FTS5 `content=''` (contentless mode) is append-only.** Triggers
  can't `DELETE` from it. Cost: one CI cycle then a switch to
  standard FTS5. Error message was clear.
- **Postgres testcontainer default is `postgres:11`.** Migration 0002
  uses `GENERATED ALWAYS AS ... STORED` columns (12+). Pinned every
  testcontainer to `pgvector/pgvector:pg17` in PR #33 (also matches
  `docker/Dockerfile.db`).
- **`cargo-deny`'s `wildcards = "deny"` flags workspace path deps**
  even with `allow-wildcard-paths = true` unless every workspace
  member sets `publish = false`. Fixed in PR #20.
- **`tokio::sync::Notify::notify_waiters()` only wakes current
  waiters.** Notification dropped if no one was waiting. Tests that
  race notify + observe must poll the state instead. Codified in
  `LoggingHandler::wait_for`.
- **pgvector `<=>` returns `[0, 2]`**, not `[0, 1]`. `1.0 - distance`
  is `-1.0` for antipodal vectors. Callers normalize if they want
  bounded ranks.
- **Indexer shutdown loop bug.** The first version of
  `Indexer::spawn` had an inner `consume()` returning unit, and the
  outer loop tried to drain the shutdown channel via `try_recv` —
  always `Empty`, so the task resubscribed forever. Fix: `consume()`
  returns a `ConsumeOutcome` enum.

## Decisions

- **Unified `Search` trait with `Unsupported` per method**, not
  separate `LexicalSearch` / `SemanticSearch` supertraits. Callers
  ask; backends answer or excuse themselves. Stays.
- **Lexical index maintenance lives in DB triggers, not in the
  indexer.** Synchronous on write — slightly slower writes, but every
  hit is fresh. Async indexing is for embeddings, where generation
  cost makes synchronous prohibitive.
- **Subscriber-side filtering on the bus.** Same as Cluster B's
  decision; no backend-specific event routing.
- **`Arc<dyn Search>` in `AppState`** unifies the HTTP and MCP
  surfaces. Same pattern as `Store`, `EventBus`, `ArtifactStore`.
- **Indexer is fire-and-forget at HTTP boundary.** `bus.publish()`
  errors are logged, never surfaced as 5xx. A degraded indexer must
  not break user-visible writes.

## Capability table extension

| Capability                                                  | First available in |
|-------------------------------------------------------------|--------------------|
| Lexical search (Postgres `tsvector` + SQLite FTS5)          | `v0.2.0`           |
| `GET /workspaces/:wid/search` HTTP route                    | `v0.2.0`           |
| MCP `search_messages` tool                                  | `v0.2.0`           |
| `<mark>`-wrapped snippet highlights                         | `v0.2.0`           |
| `pgvector` semantic search (HNSW cosine on 1024-d vectors)  | `v0.2.0`           |
| `Search::upsert_embedding` / `semantic_search`              | `v0.2.0`           |
| Bus-driven `Indexer` task with reconnect-backoff            | `v0.2.0`           |
| `EventHandler` trait + `LoggingHandler` baseline            | `v0.2.0`           |
| Cross-dialect search parity test                            | `v0.2.0`           |

## Risks identified + mitigated

- **Index drift between trigger-based and indexer-based paths.**
  Mitigated by triggers being the v0.2.0 source of truth; the
  indexer's job in this cluster is to observe, not write. When
  embeddings ship in Cluster D, the indexer becomes the writer for
  the embedding column only.
- **Wrong-dimension vectors corrupting the index.** Caught at
  `upsert_embedding` and `semantic_search` boundaries with
  `InvalidQuery` errors before any SQL runs.
- **FTS5 grammar injection via plain-language `q`.** Mitigated by
  `escape_fts5_query` which wraps every token in `"..."` phrase
  syntax so operators (`*`, `:`, `(`, `)`) become literal characters.
- **Tombstoned messages still searchable.** Mitigated at the SQL
  level: every query has `m.tombstoned_at IS NULL`. Triggers also
  remove tombstoned messages from the FTS5 index.

## Risks identified + still open

- **At-most-once indexing.** Postgres `LISTEN`/`NOTIFY` is a fire-
  and-forget transport. A subscriber that misses a notification has
  no recovery. Persistent event log in Cluster D.
- **SQLite has no semantic search.** Acceptable for dev; production
  workloads requiring vectors should use Postgres.
- **Indexer lag is invisible.** No metric exported to `/health`;
  Cluster T fixes this.
- **`v0.1.0` GitHub Release didn't auto-create** (release matrix's
  macOS x86_64 build failed because the runner is arm64-by-default).
  Need a cleanup PR before cutting `v0.2.0` that either drops the
  x86_64 darwin target or runs it via Rosetta.

## Forward look

Cluster D is the next delivery cluster: FSM-driven thread lifecycle.
Top priorities at kickoff:

1. Thread state machine (open → in_review → closed → archived) with
   typed transitions.
2. Hierarchical state machine (HSM) for nested thread workflows.
3. FSM persistence + replay from the event log.
4. Real embedding generation in the indexer (model loaded at boot,
   each `MessagePosted` produces a vector).
5. Release-workflow cleanup PR (drop or Rosetta the macOS x86_64
   build) so `v0.3.0` ships a real GitHub Release.

See [[Roadmap]] for the full ladder.

## Acknowledgements

Solo cluster. The shared `tests/common` fixture and the unified
`Search` trait continue to pay off — three backend impls (Postgres
lexical, Postgres semantic, SQLite lexical) share one assertion suite
where the contract overlaps and diverge cleanly where they don't.
