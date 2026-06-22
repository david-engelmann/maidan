# Gate: `maidan-scale-1.0`

Tagged at the same commit as **`v120.0.0`**, closing the **Product Ladder 102+**
(scale-out, hardening & correctness, search-at-scale, supply chain). This gate
does **not** regress the [`maidan-operator-1.0`](#) (`v101`),
`maidan-agent-1.0` (`v76`), or `maidan-2.0` (`v58`) contracts.

The gate is the conjunction of capabilities delivered across Clusters 102–119,
verified by the evidence below. Cluster 120 adds the gate e2e + recorded
baselines and promotes the `scale-out smoke` CI job to a required check.

## Criteria → evidence

| # | Operator can… | Clusters | Evidence |
|---|---------------|----------|----------|
| 1 | Run **≥2 replicas** behind an LB with notifications, presence, OAuth working cross-pod | 102–105 | `scale-out smoke` CI job (`scripts/scale-out-smoke.sh`, 2 replicas + shared Postgres/object store); in-process `two_replica_presence_e2e`, `two_replica_*_e2e`; Helm profiles ([[Retros/Cluster 102.0]]–[[Retros/Cluster 105.0]]) |
| 2 | Serve context/search with **bounded query counts** under load (no N+1) | 106 | `context_query_count_e2e` (query count independent of message count) |
| 3 | Tune **pool / relay / ANN** from config, with **recorded perf baselines** | 107–109 | env knobs (`MAIDAN_HNSW_M`/`_EF_CONSTRUCTION`/`_EF_SEARCH`, pool + relay config); [`SEARCH_BASELINE.md`](../../crates/maidan-search/benches/SEARCH_BASELINE.md), [`STORE_BASELINE.md`](../../crates/maidan-store/benches/STORE_BASELINE.md); [Query-Tuning.md](../Query-Tuning.md) |
| 4 | Trust one workspace **cannot starve** another | 110 | per-workspace fairness / token-quota tests ([[Retros/Cluster 110.0]]) |
| 5 | Rely on a **≥40% coverage floor**, auth + FSM directly tested, JSON-RPC surface fuzzed | 111–114 | `coverage (llvm-cov)` CI gate `COVERAGE_MIN_LINES=40` on the full suite (114); `maidan-auth` suite (111); FSM property tests (112); backend parity harness (113); JSON-RPC/MCP/A2A envelope round-trip + fuzz (114) |
| 6 | Build from a **deduplicated, advisory-clean** dependency tree | 119 | `cargo deny check` (advisories + `multiple-versions = "deny"`) in the `lint` CI job; [Dependencies.md](../Dependencies.md) |
| 7 | Pass the **`maidan-scale-1.0` gate e2e** | 120 | `maidan_scale_gate_e2e` (scale runtime surfaces + indexer lag/queue-depth gauges); `scale-out smoke` promoted to a **required** check |

## Perf budgets

Perf baselines are **machine-specific reference floors**, not absolute SLAs —
re-run on target hardware. The CI-reproducible SQLite benches establish the
floor; Postgres/pgvector latency depends on the Cluster 109 tuning knobs and
must be measured against a real instance with representative volume.

- Search: [`crates/maidan-search/benches/SEARCH_BASELINE.md`](../../crates/maidan-search/benches/SEARCH_BASELINE.md) — `cargo bench -p maidan-search --bench search_hot`.
- Store: [`crates/maidan-store/benches/STORE_BASELINE.md`](../../crates/maidan-store/benches/STORE_BASELINE.md) — `cargo bench -p maidan-store --bench store_hot`.

## Out of scope (post-gate)

Hosted SaaS / React SPA / native clients / huddles / org hierarchy (human
product); Postgres sharding / storage-engine changes (vertical + read-replica
scaling assumed sufficient). See [[Remaining Work]].

## Re-verifying the gate

```sh
cargo test -p maidan-server --test maidan_scale_gate_e2e   # gate surfaces
cargo deny check                                           # deps clean (119)
bash scripts/scale-out-smoke.sh                            # 2-replica smoke (needs Docker)
# coverage floor + multi-replica + fairness run in CI (see .github/workflows/ci.yml)
```
