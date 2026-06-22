# Cluster 120.0 retro — Scale product gate

> Tag **`v120.0.0`** / **`maidan-scale-1.0`**. Final cluster of **Phase XXIII**
> and of the entire **Product Ladder 102+** (scale-out, hardening & correctness,
> search-at-scale, supply chain).

## What shipped

- **`maidan_scale_gate_e2e`** (120.0.1): an in-process checklist asserting the
  scale runtime surfaces (`/health`, `/health/ready`, `/metrics`,
  `/openapi.json`, `/.well-known/maidan.json`) and the scale-specific telemetry
  — the indexer heartbeat + bounded-lag gauges (`maidan_indexer_queue_depth`,
  `_queue_capacity`, `_last_event_age_seconds`). Mirrors `maidan_operator_gate_e2e`.
- **`docs/Gates/maidan-scale-1.0.md`** (120.0.2): the auditable gate doc mapping
  all **7 criteria** (Clusters 102–119) to their test/CI/doc evidence, plus the
  perf-budget approach and a re-verify runbook.
- **`STORE_BASELINE.md`** (120.0.2): recorded store hot-path bench baseline
  (`sqlite_list_members_32` ≈ 0.10 ms), completing the bench-baseline pair with
  `SEARCH_BASELINE.md`.
- **`scale-out smoke` promoted** to a gate-required check (ci.yml comment + gate
  doc; the branch-protection add is the operator action noted in both).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Operator | Add `scale-out smoke` to branch-protection required checks | A repo-admin setting, not a file change; documented in the job comment + gate doc. |
| Post-gate | Hosted SaaS, React SPA, native clients, huddles, org hierarchy | Human-product, out of the 102+ scope ([[Remaining Work]] §4). |
| Post-gate | Postgres sharding / storage-engine change | Vertical + read-replica scaling assumed sufficient. |
| Track V/X | Edition 2024 adoption | Evaluated in 119; a focused migration. |

## Surprises

- **The gate is a conjunction, not new code.** Almost everything the gate
  asserts already shipped across 102–119; Cluster 120's real work was the
  *evidence map* (gate doc) + the e2e checklist + recorded baselines +
  promoting the scale-out job — making the gate auditable, not building it.
- **The in-process harness already emits the scale gauges.** `refresh_runtime_gauges`
  runs on every `/metrics` scrape and reads `state.indexer_metrics` (default
  zeros), so `maidan_indexer_queue_depth`/`_queue_capacity` are present even
  without the batching indexer wired — the gate test asserts them directly.

## Decisions

- **Gate doc as the deliverable.** A criteria→evidence table (`docs/Gates/…`)
  is the durable artifact; the e2e test is the executable smoke check. Together
  they make "does Maidan pass the scale gate?" answerable and re-runnable.
- **Recorded baselines, not SLAs.** Bench baselines are machine-specific
  reference floors (consistent with `SEARCH_BASELINE.md`); the gate records the
  CI-reproducible SQLite floors and points to the Postgres-on-target-hardware
  methodology.
- **Promotion via comment + doc, branch-protection as operator action.** The
  required-checks set is repo configuration; the cluster marks the intent and
  the operator flips branch protection (the project runs admin-merge anyway).

## Capability table extension

| Capability | Where |
|------------|-------|
| `maidan-scale-1.0` gate (criteria → evidence) | `docs/Gates/maidan-scale-1.0.md`, `maidan_scale_gate_e2e` |
| Recorded store bench baseline | `crates/maidan-store/benches/STORE_BASELINE.md` |
| `scale-out smoke` as a gate-required check | `.github/workflows/ci.yml` |

## Risks identified + still open

- **Branch-protection promotion is manual.** Until the operator adds
  `scale-out smoke` to required checks, it runs on every PR but doesn't block —
  documented so it isn't forgotten.
- **Perf budgets are reference floors.** Real SLAs need measurement on target
  hardware with representative volume (per the baseline docs).

## Forward look

**Product Ladder 102+ is complete** — gate **`maidan-scale-1.0`** at
**`v120.0.0`**, alongside `maidan-operator-1.0` (`v101`), `maidan-agent-1.0`
(`v76`), and `maidan-2.0` (`v58`). Future work is post-gate human-product and
the cross-cutting tracks (see [[Remaining Work]], [[Open Work]]); no further
ladder cluster is defined past 120.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
