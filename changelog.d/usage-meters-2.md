### Embeddings, deliveries, WASI time and stored bytes are metered

- **Added:** four more per-workspace usage meters for the free allowance (Next 27, part 2), counted into the same buffer and daily rollup as requests:
  - `embedding_calls`: each message queued for embedding by the indexer and each semantic or hybrid search query, on REST and MCP;
  - `deliveries`: each Slack, GitHub or Linear egress, each email and each web push actually sent, from the worker or inline;
  - `wasi_ms`: the milliseconds each WASI slash handler runs, for the workspace that installed it;
  - `stored_bytes`: a gauge measured once a UTC day per workspace by the `usage_meters` loop (at most 25 workspaces a tick), the bytes of the artifacts the workspace refers to plus its live message bodies. Its day's row is the latest measurement, not a sum.
