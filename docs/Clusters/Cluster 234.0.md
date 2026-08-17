# Cluster 234.0 — structured-results foundation (thread results)

> Program B (agentic orchestration), part 18. Opens **Arc F — coordination waits +
> structured results**. Phase XXIV post-gate hardening. Tag **`v234.0.0`**. No new
> gate tag.

## Goal

Open the last Program-B arc with a **zero-blast-radius foundation**: the store +
model for the structured result a task produces when it's done, and nothing wired
in yet. The DAG (222 `ThreadReady`) already tells a parent *when* a child finishes;
this is *what* it produced. REST + a `ThreadResultSet` event + `wait_for_result`
follow in 235–236.

## Scope

| Change | Where |
|--------|-------|
| `maidan_thread_results` table (pg 0041 / sqlite 0040), registered in `migrate.rs` | `migrations/{postgres,sqlite}/`, `migrate.rs` |
| `ThreadResult` model | `maidan-types/src/models.rs` |
| `Store::set_thread_result` (upsert) / `get_thread_result`, both backends | `store.rs`, `store/{sqlite,postgres}/thread_results.rs`, `store/*/mod.rs` |

## Design decisions

- **One result per thread, upserted.** `thread_id` is the PK; a re-set overwrites
  (`ON CONFLICT (thread_id) DO UPDATE`). A task has one answer; if it's revised, the
  latest wins — simpler than a versioned result log, and the event stream (235) will
  carry the "it changed" signal.
- **Arbitrary JSON.** `result` is a `serde_json::Value` — Postgres JSONB, SQLite TEXT
  (serialize on write, parse on read), the same split the Cluster-173 message
  `content` column uses. The result's shape is the agent's business, not the store's.
- **`produced_by` + `produced_at`.** Who produced it and when — audit + provenance
  for a requester aggregating sub-task results. FK-cascade on member/thread like the
  sibling tables.
- **Foundation only.** A new table + store module; zero existing paths change.

## Non-goals / deferred (the rest of Arc F)

- **REST** `PUT`/`GET /threads/:id/result` + a `ThreadResultSet` event on set
  (Cluster 235).
- **MCP** `set_thread_result` / `get_thread_result` + a **`wait_for_result`**
  long-poll (the coordination wait) + a "read my dependencies' results" aggregate for
  parent tasks (Cluster 236).

## Risks

- None — a new table off every existing path.
