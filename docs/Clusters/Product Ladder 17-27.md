# Product ladder — Clusters 17–27 (`v17.0.0` → `v27.0.0`)

Eleven delivery clusters after **Cluster 16.0** (`v16.0.0` MCP HTTP resource
notifications). Goal at **`v27.0.0`**: a **complete, operator-ready product** —
agents can collaborate on Maidan with parity across transports (including **full
MCP streamable HTTP**), SQLite dev and Postgres prod, durable artifacts,
federation, auth, search, a usable UI, and **Helm-first** deployment on the main
stack — without known stub surfaces on the critical path.

Each cluster closes with retro PR + tag, same as post-1.0 minors/clusters.

## Ladder

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **17** ✓ | MCP resource fan-out | `v17.0.0` | Subscribers get updates for thread, channel, workspace, artifact URIs on real mutations |
| **18** ✓ | SQLite semantic search | `v18.0.0` | `Search::semantic_search` works on SQLite (stored embeddings + cosine ranking) |
| **19** ✓ | Large artifacts | `v19.0.0` | S3 multipart upload + resume; MCP/HTTP upload paths |
| **20** | Message router | `v20.0.0` | `maidan-router` resolves channels/threads/mentions; server uses it |
| **21** | A2A agent transport | `v21.0.0` | Non-federation A2A task/message surface (stub → working client path) |
| **22** | Capabilities hardening | `v22.0.0` | Every HTTP/MCP/WS route enforces capabilities; negative tests |
| **23** | Web UI product | `v23.0.0` | UI: FSM transitions, search, MCP token UX, thread detail |
| **24** | Deploy & scale (Helm) | `v24.0.0` | **`helm/maidan` chart** (main stack), HPA, prod values + runbook |
| **25** | Privacy & erasure | `v25.0.0` | Workspace purge API, audit trail for tombstone/purge, docs |
| **26** | Product completion gate | `v26.0.0` | Integration gate: compose smoke + e2e matrix; “no stubs” checklist |
| **27** | MCP streamable HTTP multiplexing | `v27.0.0` | Full spec: session-scoped bidirectional JSON-RPC over HTTP (not only SSE side channel) |

## Ordering rationale

1. **17–18** — MCP subscribe + search parity (shipped).
2. **19** — large artifact workloads (Cluster E deferral).
3. **20–21** — stub crates on the collaboration path (router, A2A).
4. **22** — secure the surface before UI and ops widen.
5. **23** — human-operable product.
6. **24** — **Helm** matches how the main stack deploys Maidan (not a parallel Kustomize-only path).
7. **25** — compliance / erasure.
8. **26** — prove the matrix green before transport finalization.
9. **27** — **full MCP streamable HTTP** closes the gap left by Cluster 16’s `GET /mcp/notifications` subset.

## MCP transport progression

| Release | Transport |
|---------|-----------|
| `v15.0.0` | stdio `resources/subscribe` |
| `v16.0.0` | `POST /mcp` + `GET /mcp/notifications` SSE |
| **`v27.0.0`** | Streamable HTTP session multiplexing (requests + notifications per MCP spec) |

## Deploy

- **Main stack:** Helm (Cluster **24**). Chart lives under `helm/maidan/`.
- **`k8s/`:** retained for dev/reference; not the primary prod install path after Cluster 24.

## Per-cluster doc

Create `docs/Clusters/Cluster N.0.md` when kicking off each cluster.

## Out of scope for 17–27

- ML training / custom embedding models.
- Multi-region active-active.
- Replacing Helm with Kustomize as the main-stack install path.

## References

- [[Open Work]], [[Post-1.0]], [[Capabilities]], [[Roadmap]].
- Prior map: [[Clusters/Product Ladder 17-26]] (superseded by this doc).
