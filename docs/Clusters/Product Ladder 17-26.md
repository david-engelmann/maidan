# Product ladder — Clusters 17–26 (superseded)

> **Superseded by** [[Clusters/Product Ladder 17-27]] (adds Cluster **27** MCP streamable
> HTTP multiplexing and Helm-first Cluster **24**).

# Product ladder — Clusters 17–26 (`v17.0.0` → `v26.0.0`) — archive

Ten delivery clusters after **Cluster 16.0** (`v16.0.0` MCP HTTP resource
notifications). Goal at **`v26.0.0`**: a **complete, operator-ready product** —
agents can collaborate on Maidan with parity across transports, SQLite dev and
Postgres prod, durable artifacts, federation, auth, search, and a usable UI —
without known stub surfaces on the critical path.

Each cluster closes with retro PR + tag, same as post-1.0 minors/clusters.

## Ladder

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **17** | MCP resource fan-out | `v17.0.0` | Subscribers get updates for thread, channel, workspace, artifact URIs on real mutations |
| **18** | SQLite semantic search | `v18.0.0` | `Search::semantic_search` works on SQLite via `sqlite-vec` (dev parity) |
| **19** | Large artifacts | `v19.0.0` | S3 multipart upload + resume; MCP/HTTP upload paths |
| **20** | Message router | `v20.0.0` | `maidan-router` resolves channels/threads/mentions; server uses it |
| **21** | A2A agent transport | `v21.0.0` | Non-federation A2A task/message surface (stub → working client path) |
| **22** | Capabilities hardening | `v22.0.0` | Every HTTP/MCP/WS route enforces capabilities; negative tests |
| **23** | Web UI product | `v23.0.0` | UI: FSM transitions, search, MCP token UX, thread detail |
| **24** | Deploy & scale | `v24.0.0` | Helm chart (or equivalent), HPA manifest, prod runbook refresh |
| **25** | Privacy & erasure | `v25.0.0` | Workspace purge API, audit trail for tombstone/purge, docs |
| **26** | Product completion gate | `v26.0.0` | Integration gate: compose smoke + e2e matrix; “no stubs” checklist; `v2.6.0` product retro |

## Ordering rationale

1. **17** builds on 15–16 MCP subscribe — highest leverage for agents without new infra.
2. **18** closes the longest-standing dev/prod search gap (Open Work).
3. **19** unblocks real artifact workloads (Cluster E deferral).
4. **20–21** replace stub crates on the collaboration path (router, A2A).
5. **22** secures the surface before UI and ops widen (Cluster F depth).
6. **23** makes the product human-operable, not only agent-API.
7. **24–25** production and compliance expectations for “complete”.
8. **26** is deliberate consolidation — not new features; prove the matrix green.

## Per-cluster doc

Create `docs/Clusters/Cluster N.0.md` when kicking off each cluster (copy 16.0
template). This file is the **epic map** only.

## Out of scope for 17–26

- Full MCP streamable HTTP multiplexing (optional post-26).
- Replacing Kustomize *and* shipping Helm (pick one in 24).
- ML training / custom embedding models.
- Multi-region active-active.

## References

- [[Open Work]], [[Post-1.0]], [[Capabilities]], [[Roadmap]].
- Stubs: `maidan-router`, partial `maidan-a2a`, SQLite semantic gap.
