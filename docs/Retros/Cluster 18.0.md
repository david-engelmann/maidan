# Cluster 18.0 retro — SQLite semantic search

> Closing wave for Cluster 18.0 · target tag `v18.0.0`.

Cluster 18.0 enabled `Search::semantic_search` on SQLite for dev deployments.

## What shipped

- **PR #188** — SQLite migration `0003_embeddings`, `upsert_embedding`, cosine-ranked
  `semantic_search`, tests + HTTP `mode=semantic` on SQLite.

## What was deferred

| To          | What                                    | Why                                      |
|-------------|-----------------------------------------|------------------------------------------|
| Cluster 19  | S3 multipart                            | Separate artifact epic.                  |
| Cluster 27  | Full MCP streamable HTTP multiplexing   | Transport finalization after core product. |
| Post-18.0   | `sqlite-vec` SQL functions              | sqlx/extension linkage unreliable.       |

## Forward look

Next: **Cluster 19.0** — large artifacts (multipart). Ladder: [[Clusters/Product Ladder 17-27]].
