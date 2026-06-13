# Query tuning playbook (Track U.3)

How to investigate slow Postgres paths in Maidan. SQLite dev deployments
use FTS5; most production installs use Postgres + `pgvector`.

## Event log replay

```sql
EXPLAIN (ANALYZE, BUFFERS)
SELECT id, workspace_id, kind, payload, created_at
FROM maidan_events
WHERE workspace_id = $1 AND id > $2
ORDER BY id ASC
LIMIT 100;
```

Expect index range scan on `(workspace_id, id)` if migration indexes are present.

## Lexical search

```sql
EXPLAIN (ANALYZE, BUFFERS)
SELECT m.id, m.thread_id, m.body,
       ts_rank_cd(m.search_vec, plainto_tsquery('english', $2)) AS rank
FROM maidan_messages m
JOIN maidan_threads t ON t.id = m.thread_id
JOIN maidan_channels c ON c.id = t.channel_id
WHERE c.workspace_id = $1
  AND m.tombstoned_at IS NULL
  AND m.search_vec @@ plainto_tsquery('english', $2)
ORDER BY rank DESC
LIMIT 25;
```

If seq scans appear at scale, verify the GIN index `idx_messages_search_vec` on
`search_vec` and run `ANALYZE maidan_messages`.

## Semantic search (pgvector)

```sql
EXPLAIN (ANALYZE, BUFFERS)
SELECT message_id, embedding <=> $2::vector AS distance
FROM maidan_emb_hash_v1
ORDER BY distance
LIMIT 25;
```

Embedding storage is per-model (registry `maidan_embedding_models`, one table per
model — `maidan_emb_hash_v1` for the default provider); query the table for the
active model. HNSW index should appear for cosine distance. Rebuild or tune `ef_search` if
recall drops after bulk ingest.

## Context assembly (bulk reads, `v106.0.0`)

Thread and workspace context are assembled with **batched** store reads, not one
query per row. Each `build_thread_context` issues a fixed set of queries
regardless of how many messages the thread has:

- threads for a workspace: one `list_threads_for_workspace` (a `threads ⋈
  channels` join on `workspace_id`) instead of one `list_threads` per channel;
- references: one `list_references_from_many(Message, ids)` (`src_id = ANY($1)`
  on Postgres; chunked `IN (?, …)` on SQLite) instead of one read per message;
- edits: one windowed `list_message_edits_for_messages(ids, 20)`
  (`ROW_NUMBER() OVER (PARTITION BY message_id …)`, capped per message) instead
  of one read per message.

`context_query_count_e2e` guards this: it counts `sqlx::query` tracing events and
asserts a 40-message thread issues the same query count as a 3-message one. If
you add a call site that reads per-row in a loop, batch it the same way (add a
concrete `…_many` accessor) rather than looping — the test will flag the
regression.

```sql
-- Verify the workspace-threads join uses the channel index, not a seq scan.
EXPLAIN (ANALYZE, BUFFERS)
SELECT t.id FROM maidan_threads t
JOIN maidan_channels c ON c.id = t.channel_id
WHERE c.workspace_id = $1
ORDER BY t.created_at DESC;
```

Note: per-thread sub-context in workspace context is still O(threads) (each
thread is its own bounded context), and artifact metadata reads are still
per-distinct-sha — both are out of the `v106.0.0` scope.

## When to escalate

- p95 search latency grows after indexer backlog — check `/health/ready`
  `indexer_last_event_at` and `INDEXER_STALE_SECS`.
- NOTIFY gaps — clients should use WS `replay_hint` + `GET …/events` (shipped
  in `v1.1.0`).

See [[Open Work]] for perf track items (criterion benches, mutation tests).
