# Cluster C — Search + indexing

After Cluster B made the workspace reachable, Cluster C makes it
searchable. Full-text first, vector second, async indexer last.

> **Goal:** `GET /workspaces/:wid/search?q=...` returns ranked
> message hits with snippets. The same surface is reachable as an
> MCP tool. `pgvector` powers semantic search alongside lexical.
>
> **Target tag:** `v0.2.0`.

## PRs

| #   | Title                                                            | Issue |
|-----|------------------------------------------------------------------|-------|
| C.1 | `feat(maidan-search): full-text search (postgres tsvector + sqlite fts5)` | #26 |
| C.2 | `feat(maidan-server): http /search + mcp search_messages tool`   | #27   |
| C.3 | `feat(maidan-search): pgvector embeddings + semantic search`     | #28   |
| C.4 | `feat(maidan-search): bus-driven background indexer`             | #29   |
| C.retro | `docs(retro): Cluster C retrospective + v0.2.0 tag prep`     | #30   |

## Order

1. **C.1 first** — schema + Search trait + both backends. Synchronous
   triggers on Postgres / sync sidecar table on SQLite keep the index
   current until C.4 lands.
2. **C.2** wires search to the HTTP + MCP surfaces.
3. **C.3** adds vector search on top of `pgvector` (already bundled in
   `docker/Dockerfile.db`).
4. **C.4** replaces synchronous indexing with a bus-subscribed task,
   which is the path embedding generation will take in Cluster D.
5. **C.retro** closes the cluster + cuts `v0.2.0`.

## Exit criteria

- CI green on `main`.
- Lexical search hits within 100 ms on a 10 k-message workspace.
- Semantic search (Postgres) hits via HNSW index.
- Background indexer keeps lexical and embedding indexes current within
  500 ms of a `MessagePosted` event.
- [[Retros/Cluster C]] merged.
- `v0.2.0` tagged.

## Risks

| Risk                                                              | Mitigation                                                                |
|-------------------------------------------------------------------|---------------------------------------------------------------------------|
| `ts_headline` slow on large messages                              | Cap body length read by the snippet function; defer to streaming if hit.  |
| `pgvector` dimension churn between models                         | Store the model name with every vector; require model match on search.    |
| SQLite has no native vector type                                  | Lexical-only on SQLite for v0.2.0; document `Search::semantic_search` returns `Unsupported`. `sqlite-vec` extension is a Cluster F+ candidate. |
| Background indexer falls behind under burst load                  | Lag metric on `/health`; bounded queue with shed-old-events under press.  |
| Synchronous trigger doubles every write                           | Acceptable until C.4; sidecar background indexer drops the trigger then.  |
