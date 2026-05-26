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
       ts_rank(m.search_vector, plainto_tsquery('english', $2)) AS rank
FROM maidan_messages m
JOIN maidan_threads t ON t.id = m.thread_id
JOIN maidan_channels c ON c.id = t.channel_id
WHERE c.workspace_id = $1
  AND m.tombstoned_at IS NULL
  AND m.search_vector @@ plainto_tsquery('english', $2)
ORDER BY rank DESC
LIMIT 25;
```

If seq scans appear at scale, verify GIN index on `search_vector` and run
`ANALYZE maidan_messages`.

## Semantic search (pgvector)

```sql
EXPLAIN (ANALYZE, BUFFERS)
SELECT message_id, embedding <=> $2::vector AS distance
FROM maidan_message_embeddings
ORDER BY distance
LIMIT 25;
```

HNSW index should appear for cosine distance. Rebuild or tune `ef_search` if
recall drops after bulk ingest.

## When to escalate

- p95 search latency grows after indexer backlog — check `/health/ready`
  `indexer_last_event_at` and `INDEXER_STALE_SECS`.
- NOTIFY gaps — clients should use WS `replay_hint` + `GET …/events` (shipped
  in `v1.1.0`).

See [[Open Work]] for perf track items (criterion benches, mutation tests).
