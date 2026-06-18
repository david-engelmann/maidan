# Product Ladder 102+ — Scale-out, hardening & correctness

**North star:** A single Maidan deployment runs **correctly across N replicas** under real load, with the known hot paths bounded, the most safety-critical crates actually tested, and search/indexing that scales to large workspaces — without regressing the **`maidan-agent-1.0`** or **`maidan-operator-1.0`** contracts. The theme is *depth, not surface*: make what already ships production-grade at scale.

**Predecessor:** [[Clusters/Product Ladder 77+]] closed at **`v101.0.0`** / **`maidan-operator-1.0`**.

**Versioning:** One cluster → one tag (`v102.0.0` … `v120.0.0`). Product gate after Cluster **120**: tag **`maidan-scale-1.0`** at the same commit as **`v120.0.0`** (distinct from **`maidan-operator-1.0`**, **`maidan-agent-1.0`**, **`maidan-2.0`**, semver **`v1.0.0`**).

> [!note] Why this ladder exists
> The codebase is disciplined and feature-complete for the agent + operator surface. The remaining risk is **operational**, not functional: the server keeps load-bearing state in-process (so >1 replica misbehaves), a few endpoints issue N+1 queries, the two most critical crates lean on inline tests, and search degrades on large workspaces. This ladder closes that gap.

---

## Honest gap register (post-101)

| Area | At **`v101.0.0`** | This ladder |
|------|-------------------|-------------|
| **Horizontal scale** | Per-process in-memory state: MCP subscriptions/notifications (`maidan-mcp/src/server.rs`), presence/typing (`presence.rs`), app-OAuth codes (`app_oauth.rs`), reindex registry (`reindex_ops.rs`) | **102–105** — externalize to a shared fabric; correct under ≥2 replicas |
| **Context endpoints** | N+1: per-channel thread reads + per-message refs/edits (`thread_context.rs:89–119,148`) | **106** — bulk/batched store reads |
| **DB pool** | Hardcoded 16 (PG) / 8 (SQLite), no acquire/statement timeout (`main.rs:58`) | **107** — configurable pool + timeouts |
| **Outbox relay** | Fixed 50 ms busy-poll (`outbox_relay.rs`) | **108** — adaptive / NOTIFY-driven relay |
| **ANN index** | pgvector HNSW created with defaults, untuned (`embedding_tables.rs:105`) | **109** — exposed `m`/`ef_*` params + bench harness |
| **Tenant fairness** | Rate limits + quotas are per-token only | **110** — per-workspace query/indexer fairness |
| **Auth tests** | Inline `#[cfg(test)]` only; no `tests/` suite for `maidan-auth` | **111** — dedicated authZ/crypto suite |
| **FSM tests** | Inline only for `maidan-fsm` | **112** — exhaustive `proptest` over transitions + HSM |
| **Backend parity** | Postgres/SQLite hand-maintained in parallel (~7.4k LOC), no sync check | **113** — parity harness + CI file-parity guard |
| **Coverage** | Floor **11%** (`ci.yml`) | **114** — staged uplift + MCP/JSON-RPC fuzz |
| **God modules** | `routes.rs` 1.6k LOC, `tools.rs` 1.4k LOC; ~2 non-test `unwrap()` | **115** — split by domain + `unwrap()` purge |
| **Embedding throughput** | One async indexer, per-event embed call | **116** — batch pipeline + large-workspace queue |
| **Embedding quality** | `hash-v1` default (deterministic, not semantic) | **117** — pluggable production provider + per-model path |
| **Search relevance** | Lexical and semantic returned separately | **118** — hybrid ranking + relevance eval harness |
| **Dependency hygiene** | Duplicate majors (`thiserror` 1+2, `hmac`, `base64`); `rsa` advisory via `openidconnect` 4.x | **119** — dedupe + currency |
| **Scale gate** | **`maidan-operator-1.0`** | **120** — **`maidan-scale-1.0`** |

---

## Ladder overview

| Phase | Clusters | Tags | Theme |
|-------|----------|------|--------|
| **XIX — Scale-out core** | 102–105 | `v102`–`v105` | Externalize per-process state; multi-replica correctness |
| **XX — Hot-path hardening** | 106–110 | `v106`–`v110` | Kill N+1s, configurable pool, adaptive relay, ANN tuning, tenant fairness |
| **XXI — Correctness & coverage** | 111–115 | `v111`–`v115` | Auth + FSM tests, backend parity, coverage uplift, module split |
| **XXII — Search & indexer at scale** | 116–118 | `v116`–`v118` | Batch embeddings, pluggable provider, hybrid relevance |
| **XXIII — Supply chain & scale gate** | 119–120 | `v119`–`v120` | Dependency dedupe/currency, **`maidan-scale-1.0`** |

---

## Phase XIX — Scale-out core (Clusters 102–105)

> [!warning] The load-bearing limitation
> The event **log + bus** are multi-process-safe (LISTEN/NOTIFY), but the server keeps **notifications, presence, OAuth codes, and reindex jobs in-process memory**. Behind a load balancer with >1 replica: MCP notifications only reach same-pod subscribers, presence is invisible cross-pod, and an OAuth code minted on pod A is lost if the exchange lands on pod B. This phase is the prerequisite for every other scale claim.

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **102** | Cross-pod notification fabric | `v102.0.0` | MCP `notifications/resources/updated` + streamable sessions delivered across replicas via Postgres LISTEN/NOTIFY (not per-process broadcast); 2-replica notification e2e |
| **103** | Distributed presence & roster | `v103.0.0` | `PresenceHub` backed by a shared channel (NOTIFY or Redis); presence + typing visible across pods; roster consistent under multi-replica |
| **104** | Durable ephemeral state | `v104.0.0` | App-OAuth codes and reindex job registry persisted (survive pod hop / restart); no in-memory-only state on the auth request path |
| **105** | Multi-replica scale-out smoke | `v105.0.0` | CI job runs core e2e against **≥2 server replicas** behind an LB; supported replica topology documented in [[Production]] |

**Ordering:** **102** establishes the cross-pod fabric pattern that **103** reuses. **104** independent. **105** after 102–104 (it proves them).

---

## Phase XX — Hot-path hardening (Clusters 106–110)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **106** | Bulk store reads | `v106.0.0` | `list_threads_for_workspace` + batched references/edits (`= ANY($1)`); context endpoints issue O(1) round-trips, not O(channels)/O(messages); regression test asserts query count |
| **107** | Pool & timeouts configurable | `v107.0.0` | Env-driven `max_connections`, `acquire_timeout`, statement timeout; documented defaults in [[Production]] |
| **108** | Adaptive outbox relay | `v108.0.0` | Relay reacts to NOTIFY / backs off when idle instead of fixed 50 ms; catch-up latency bounded under burst; metrics unchanged |
| **109** | ANN index tuning + bench | `v109.0.0` | HNSW `m`/`ef_construction`/`ef_search` configurable; `criterion` bench harness for lexical + semantic latency (Track **U**) |
| **110** | Per-workspace fairness | `v110.0.0` | Per-workspace query/indexer budget so one tenant cannot starve search/indexing for others; documented limits |

**Ordering:** **106** independent, highest ROI — do first. **109** before the **120** perf budgets. **110** after **109** (needs the bench baseline).

---

## Phase XXI — Correctness & coverage (Clusters 111–115)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **111** ✓ | Auth test suite | `v111.0.0` ✓ | `maidan-auth/tests/`: capability resolution matrix, token lifecycle/revocation, peer-secret AEAD round-trip + tamper, constant-time paths |
| **112** ✓ | FSM property tests | `v112.0.0` ✓ | `proptest` proving only legal `(state, action)` edges succeed, `archived` terminal, and the HSM child-rank ≤ parent invariant holds for arbitrary trees |
| **113** ✓ | Backend parity harness | `v113.0.0` ✓ | Shared assertion suite both backends must pass + CI guard that `migrations/postgres` ↔ `migrations/sqlite` and store modules stay in lockstep |
| **114** ✓ | Coverage uplift + fuzz | `v114.0.0` ✓ | `COVERAGE_MIN_LINES` raised in steps (11 → 25 → 40); fuzz/round-trip tests on the JSON-RPC / MCP envelope surface |
| **115** ✓ | Module split + `unwrap()` purge | `v115.0.0` ✓ | `routes.rs` and `tools.rs` split by domain; zero non-test `unwrap()`/`expect()` in `crates/*/src/`; clippy lint added to enforce |

**Ordering:** **111** + **112** independent (start immediately — both are pure-logic crates). **113** before/with **114**. **115** can interleave (mechanical).

---

## Phase XXII — Search & indexer at scale (Clusters 116–118)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **116** ✓ | Batch embedding pipeline | `v116.0.0` ✓ | Indexer batches embed calls with backpressure; large-workspace backfill runs on a separate queue so live indexing stays fresh; indexer-lag metric bounded |
| **117** ✓ | Pluggable production provider | `v117.0.0` ✓ | First-class `openai-compatible` provider path with tunable dimension/model, slotting into the per-model table scheme (v47); migration/reindex story documented |
| **118** | Hybrid relevance | `v118.0.0` | Optional hybrid lexical+semantic ranking over the normalized `[0,1]` score; a small relevance eval harness guards regressions |

**Ordering:** **116** before **117** (batching makes a remote provider viable). **118** after **117**.

---

## Phase XXIII — Supply chain & scale gate (Clusters 119–120)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **119** | Dependency dedupe & currency | `v119.0.0` | Collapse duplicate majors (`thiserror`/`hmac`/`base64`); tighten `deny.toml` `multiple-versions` for crypto crates; track `openidconnect` v5 to clear the `rsa` advisory; evaluate edition 2024 (Track **V/X**) |
| **120** | Scale product gate | `v120.0.0` | **`maidan-scale-1.0`** e2e: multi-replica suite (102–105) + perf budgets (109) + coverage floor (114) + clean `cargo deny` + bench baselines recorded |

**Ordering:** **119** anytime (Track **V**). **120** last.

---

## Dependency sketch

```text
102 (notif fabric) ──┐
103 (presence) ← 102 │
104 (durable state)  ├─→ 105 (multi-replica smoke) ─────────────┐
                     │                                          │
106 (bulk reads) ────┤  (independent, do first)                 │
107 (pool cfg)       │                                          ├─→ 120 (maidan-scale-1.0)
108 (adaptive relay) │                                          │
109 (ANN bench) ──────────→ 110 (fairness) ─────→ perf budgets ─┤
                     │                                          │
111 (auth) 112 (fsm) ├─→ 113 (parity) → 114 (coverage) ─────────┤
115 (split/unwrap)   │                                          │
116 (batch) → 117 (provider) → 118 (hybrid) ────────────────────┘
119 (deps) ──────────┘
```

**Parallelizable immediately:** 106 + 111 + 112 + 115 + 119 (no cross-deps).

---

## Per-cluster kickoff docs

Create `docs/Clusters/Cluster N.0.md` when starting each cluster (copy the [[Clusters/Cluster 101.0]] / [[Clusters/Cluster 68.0]] template). This file is the **epic map** only.

| Cluster | Kickoff doc |
|---------|-------------|
| **102** | [[Clusters/Cluster 102.0]] |
| **103** | [[Clusters/Cluster 103.0]] |
| **104** | [[Clusters/Cluster 104.0]] |
| **105** | [[Clusters/Cluster 105.0]] |
| **106** | [[Clusters/Cluster 106.0]] |
| **107** | [[Clusters/Cluster 107.0]] |
| **108** | [[Clusters/Cluster 108.0]] |
| **109** | [[Clusters/Cluster 109.0]] |
| **110** | [[Clusters/Cluster 110.0]] |
| **111** | [[Clusters/Cluster 111.0]] |
| **112** | [[Clusters/Cluster 112.0]] |
| **113** | [[Clusters/Cluster 113.0]] |
| **114** | [[Clusters/Cluster 114.0]] |
| **115** | [[Clusters/Cluster 115.0]] |
| **116** | [[Clusters/Cluster 116.0]] |
| **117** | [[Clusters/Cluster 117.0]] |
| **118** | [[Clusters/Cluster 118.0]] |
| **119** | [[Clusters/Cluster 119.0]] |
| **120** | [[Clusters/Cluster 120.0]] |

---

## Explicitly not in 102–120

| Item | Why / where |
|------|-------------|
| Multi-region active-active | Out of scope ([[Open Work]]); this ladder is single-region, multi-replica |
| SAML / SCIM inside Maidan | [[OIDC]] non-goals |
| React SPA / native clients / huddles / org hierarchy | Post-gate human-product ([[Remaining Work]] §4) |
| Sharding Postgres / changing the storage engine | Vertical + read-replica scaling assumed sufficient pre-gate |
| Rewriting the dual-backend store as one abstraction | **113** adds a parity *guard*, not a rewrite |
| Spec-complete MCP streamable mux | Already tracked from **78**; this ladder consumes it, doesn't redo it |

---

## Product gate: `maidan-scale-1.0`

At **`v120.0.0`**, an operator can:

1. Run **≥2 server replicas** behind a load balancer with notifications, presence, and OAuth working across pods (**102–105**).
2. Serve context/search endpoints with **bounded query counts** under load (**106**, no N+1).
3. Tune pool, relay, and ANN parameters from config, with **recorded perf baselines** (**107–109**).
4. Trust that one workspace **cannot starve** another (**110**).
5. Rely on a **≥40% coverage floor** with the auth and FSM crates directly tested and the JSON-RPC surface fuzzed (**111–114**).
6. Build from a **deduplicated, advisory-clean** dependency tree (**119**).
7. Pass the **`maidan-scale-1.0`** gate e2e (**120**).

This ladder **does not regress** **`maidan-agent-1.0`** (`v76`) or **`maidan-operator-1.0`** (`v101`) contracts.

---

## References

- [[Clusters/Product Ladder 77+]], [[Retros/Cluster 101.0]]
- [[Roadmap]], [[Architecture]], [[Production]], [[Decisions]]
- [[Open Work]], [[Remaining Work]], [[Threat-Model]]
