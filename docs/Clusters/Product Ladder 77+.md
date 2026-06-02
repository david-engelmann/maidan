# Product Ladder 77+ — Operator production & UI v1

**North star:** Close post–**`maidan-agent-1.0`** debt so a human operator can run Maidan in production with confidence (observability, delivery ops, Helm profiles) and a minimal **`/ui`** for day-two tasks — while finishing transport and search gaps left from [[Clusters/Product Ladder 68+]].

**Predecessor:** [[Clusters/Product Ladder 68+]] closed at **`v76.0.0`** / **`maidan-agent-1.0`**.

**Versioning:** One cluster → one tag (`v77.0.0` … `v101.0.0`). Product gate after Cluster **101**: tag **`maidan-operator-1.0`** at the same commit as **`v101.0.0`** (distinct from **`maidan-agent-1.0`** and semver **`v1.0.0`**).

---

## Honest gap register (post–76)

| Area | At **`v76.0.0`** | This ladder |
|------|------------------|-------------|
| HTTP capability CI | MCP matrix + sample HTTP contract (**69**) | **77** — every OpenAPI operation |
| MCP streamable | POST/DELETE + TTL (**73**) | **78** — bidirectional mux subset + client doc |
| A2A tasks | Persisted tasks + `SubscribeToTask` (**72**) | **79** — cancel, progress, long-running semantics |
| Delivery ops | Separate webhook + automation DLQs (**68**) | **80** — unified operator list/filters |
| WS subscribe | Filter schema (**71**) | **81** — grant-scoped channel lists |
| Context export | MCP + HTTP tools (**74**, **67**) | **82** — pagination cursors |
| SQLite parity | Delivery cursor no-op | **83** |
| Outbox | Relay + quarantine; NOTIFY at-most-once | **84** — polled relay mode or documented upgrade |
| Semantic SQLite | Brute-force cosine (**75**) | **85** — optional `sqlite-vec` |
| Embeddings | Single table + `hash-v1` default | **86** — per-model tables |
| Reindex | CLI `maidan reindex-embeddings` (**75**) | **87** — operator HTTP job API |
| Helm | Stack + kind CI (**55**) | **88** — production values profiles |
| Metrics | Runbook + gate e2e (**76**) | **89–90** — OTLP export + alert templates |
| Bootstrap | Runtime `MAIDAN_BOOTSTRAP` gate | **91** — compile-time strip |
| Human UI | Tabbed `/ui` (**23**) | **92–96** — channels, live tail, artifacts, search, tokens |
| Collaboration | 1:1 DMs (**39**) | **97–99** — group DMs, mentions, presence |
| mcp-stdio | Postgres-only, no indexer | **100** |
| Product gate | **`maidan-agent-1.0`** | **`maidan-operator-1.0`** at **101** |

---

## Ladder overview

| Phase | Clusters | Tags | Theme |
|-------|----------|------|--------|
| **XIV — Contract & transport** | 77–81 | `v77`–`v81` | HTTP map, streamable mux, A2A depth, delivery ops, subscribe grants |
| **XV — Store & memory** | 82–86 | `v82`–`v86` | Context cursors, SQLite parity, outbox mode, semantic scale |
| **XVI — Platform ops** | 87–91 | `v87`–`v91` | Reindex API, Helm profiles, OTLP, SLO alerts, bootstrap strip |
| **XVII — Operator UI v1** | 92–96 | `v92`–`v96` | Channels, live events, artifacts, search, token admin |
| **XVIII — Collaboration & gate** | 97–101 | `v97`–`v101` | Group DMs, mentions, presence, stdio embed, operator gate |

---

## Phase XIV — Contract & transport (Clusters 77–81)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **77** | HTTP capability map complete | `v77.0.0` ✓ | `contracts/http-capability-map.json` + OpenAPI parity CI + `http_capability_matrix_e2e` |
| **78** | MCP streamable bidirectional | `v78.0.0` | Multiplexed JSON-RPC over streamable session per documented 2024-11-05 subset; client example in [[Agent Integration]] |
| **79** | A2A long-running tasks | `v79.0.0` | `tasks/cancel`, progress events on `SubscribeToTask`, terminal semantics tested |
| **80** | Delivery ops unified | `v80.0.0` | Single operator API shape to list/replay webhook + automation deliveries (tables may stay separate) |
| **81** | Subscribe grants v3 | `v81.0.0` | WS filter schema requires explicit channel grants; private-channel deny e2e |

**Ordering:** **77** before **81** (capabilities reference routes). **78** parallel after **77**. **79** after **72** baseline. **80** after **68**.

---

## Phase XV — Store & memory (Clusters 82–86)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **82** | Context pagination | `v82.0.0` | HTTP + MCP context tools accept cursor/limit; stable ordering documented |
| **83** | SQLite delivery cursor | `v83.0.0` | `maidan_delivery_cursor` implemented for SQLite; shared store tests |
| **84** | Outbox relay modes | `v84.0.0` | Configurable polled relay + runbook for NOTIFY loss; no silent downgrade in prod |
| **85** | sqlite-vec optional | `v85.0.0` | Feature-gated HNSW on SQLite; CI job proves linkage or documents opt-out |
| **86** | Per-model embeddings | `v86.0.0` | Schema split by `embedding_model`; queries filter by model at index time |

**Ordering:** **82** anytime after **74**. **83** independent. **85** before **86**. **87** (next phase) after **86**.

---

## Phase XVI — Platform ops (Clusters 87–91)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **87** | Reindex job API | `v87.0.0` | `POST /operator/.../reindex-embeddings` + job status; complements CLI |
| **88** | Helm production profiles | `v88.0.0` | Documented values overlays: external OTel, Redis quotas, S3, ingress TLS |
| **89** | OTLP metrics export | `v89.0.0` | Prometheus scrape or OTLP push from server; example dashboard JSON in repo |
| **90** | SLO alert templates | `v90.0.0` | Grafana/Alertmanager YAML for agent latency, DLQ depth, outbox lag |
| **91** | Bootstrap compile-time strip | `v91.0.0` | Release build without bootstrap routes via Cargo feature; threat model updated |

**Ordering:** **89** → **90**. **88** parallel. **91** anytime (Track V). **87** after **86**.

---

## Phase XVII — Operator UI v1 (Clusters 92–96)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **92** | `/ui` channels | `v92.0.0` | List channels, open thread, post message without curl |
| **93** | `/ui` live events | `v93.0.0` | WS subscribe panel with filter presets + reconnect |
| **94** | `/ui` artifacts | `v94.0.0` | Multipart upload + artifact card in thread view |
| **95** | `/ui` search | `v95.0.0` | Faceted search UI matching HTTP search API |
| **96** | `/ui` tokens & apps | `v96.0.0` | List/rotate capability tokens; read-only installed apps |

**Ordering:** **92** before **93–94**. **95** after search API stable. **96** after **65** OAuth baseline. No React SPA in this ladder — extend existing server-rendered `/ui`.

---

## Phase XVIII — Collaboration & gate (Clusters 97–101)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **97** | Group DMs | `v97.0.0` | Multi-member DM threads + capability enforcement + tests |
| **98** | Mention notifications | `v98.0.0` | Mention → webhook router (config per workspace); no email required |
| **99** | Presence v2 | `v99.0.0` | Workspace roster HTTP + WS presence fanout documented |
| **100** | mcp-stdio embedded | `v100.0.0` | Optional single-binary mode: store + indexer in-process for demos |
| **101** | Operator product gate | `v101.0.0` | **`maidan-operator-1.0`** e2e: UI smoke + HTTP map + metrics scrape + runbook checklist |

**Ordering:** **97** independent. **98** after **50** webhooks. **101** last.

---

## Dependency sketch

```text
77 (HTTP map) ──────────────────────────┐
78 (streamable) ← 77                    │
79 (A2A) ← 72                           │
80 (delivery ops) ← 68                  ├→ 101 (maidan-operator-1.0)
81 (subscribe) ← 77                   │
82 (context) ← 74                       │
85 (sqlite-vec) → 86 → 87               │
89 (OTLP) → 90                          │
92 (ui channels) → 93, 94               │
```

**Parallelizable after 77:** 78 + 79 + 83 + 88 + 91.

---

## Per-cluster kickoff docs

Create `docs/Clusters/Cluster N.0.md` when starting each cluster (copy [[Clusters/Cluster 68.0]] template). This file is the **epic map** only.

| Cluster | Kickoff doc |
|---------|-------------|
| **77** | [[Clusters/Cluster 77.0]] |
| **78** | [[Clusters/Cluster 78.0]] |
| **79** | [[Clusters/Cluster 79.0]] |
| **80** | [[Clusters/Cluster 80.0]] |
| **81** | [[Clusters/Cluster 81.0]] |
| **82** | [[Clusters/Cluster 82.0]] |
| **83** | [[Clusters/Cluster 83.0]] |
| **84** | [[Clusters/Cluster 84.0]] |
| **85** | [[Clusters/Cluster 85.0]] |
| **86** | [[Clusters/Cluster 86.0]] |
| **87** | [[Clusters/Cluster 87.0]] |
| **88** | [[Clusters/Cluster 88.0]] |
| **89** | [[Clusters/Cluster 89.0]] |
| **90** | [[Clusters/Cluster 90.0]] |
| **91** | [[Clusters/Cluster 91.0]] |
| **92** | [[Clusters/Cluster 92.0]] |
| **93** | [[Clusters/Cluster 93.0]] |
| **94** | [[Clusters/Cluster 94.0]] |
| **95** | [[Clusters/Cluster 95.0]] |
| **96** | [[Clusters/Cluster 96.0]] |
| **97** | [[Clusters/Cluster 97.0]] |
| **98** | [[Clusters/Cluster 98.0]] |
| **99** | [[Clusters/Cluster 99.0]] |
| **100** | [[Clusters/Cluster 100.0]] |
| **101** | [[Clusters/Cluster 101.0]] |

---

## Explicitly not in 77–101

| Item | Track |
|------|--------|
| Multi-region active-active | [[Open Work]] |
| SAML/SCIM inside Maidan | [[OIDC]] non-goals |
| React SPA / design-system rewrite | Post–**101** human-product |
| Huddles, Slack Connect UX, org hierarchy | [[Remaining Work]] §4 |
| Sigstore/cosign | May land in **91** or Track **X** opportunistically |
| Merge webhook + automation DB tables | **80** unifies operator API only |

---

## Product gate: `maidan-operator-1.0`

At **`v101.0.0`**, a platform operator can:

1. Prove every HTTP route is capability-gated in CI (**77**).
2. Run with exported metrics and documented alerts (**89–90**).
3. Operate delivery failures from UI or unified API (**80**, **92–96**).
4. Reindex embeddings without shell access (**87**).
5. Deploy from Helm profiles with OTel + Redis (**88**).
6. Pass **`maidan-operator-1.0`** gate e2e (**101**).

Agents retain **`maidan-agent-1.0`** guarantees from **`v76.0.0`**; this ladder does not regress agent contracts.

---

## References

- [[Clusters/Product Ladder 68+]], [[Retros/Cluster 76.0]], [[Agent Integration]]
- [[Remaining Work]], [[Open Work]], [[Roadmap]], [[Production]]
