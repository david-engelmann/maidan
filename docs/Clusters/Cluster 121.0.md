# Cluster 121.0 — Observability & contract completeness

**Theme:** Close two named, owner-less backlog gaps — the OpenAPI-wide
capability map in CI (deferred since Cluster 69) and scale-out SLO
dashboard/alert coverage for the Cluster 116 indexer metrics.

**Ladder:** Post-gate — **Phase XXIV**, the first cluster after Product
Ladder 102+ closed at `maidan-scale-1.0` (`v120.0.0`). Tag **`v121.0.0`**;
no new gate tag (this is hardening, not a product gate).

**Predecessor:** Cluster 69 (capability matrix + sample HTTP contract),
Cluster 90 (SLO recording/alert rules + operator dashboard), Cluster 116
(batched-embed indexer + its queue/embed gauges).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Contract (121.0.1)** | `every_openapi_operation_is_bearer_session_or_public` — classify every OpenAPI op as bearer-mapped, session-cookie-gated, or explicitly public; fail CI on an unclassified route. |
| **Alerts (121.0.2)** | `maidan_slo:indexer_queue_saturation` recording rule; `MaidanIndexerQueueSaturated` + `MaidanIndexerEmbedFailures` alerts. |
| **Dashboard (121.0.2)** | Operator dashboard panels: indexer queue depth vs capacity; embed failures total. |
| **Contract (121.0.2)** | `alert_templates_contract` `expected[]` asserts the three new indexer metric names. |
| **Docs** | `Remaining Work.md` §1/§3 — close the capability-map gap; note SLO coverage extended to scale-out metrics. |

## Non-goals

- OTLP export wiring — this cluster covers the dashboards/alerts half only.
  (_Correction, Cluster 122: the OTLP exporter itself already shipped in
  Cluster 89; it was never an open deferral._)
- New product capability — both items are completeness, not features.
- `rate()`/`increase()` on the indexer gauges — they are monotonic gauge
  mirrors, not Prometheus counters; alerts use ratio + offset-delta.

## PR ladder (actual)

| # | Title |
|---|--------|
| 121.0.1–2 | `feat: OpenAPI op classification + scale-out SLO coverage` (#333) |
| 121.0.retro | `docs(retro): Cluster 121.0 + v121.0.0 tag prep` |

## Exit criteria

- Every OpenAPI op is classified in CI; a new unauthenticated/unmapped
  route fails the contract — **met**.
- The Cluster 116 indexer queue/embed metrics have recording rule + alerts
  + dashboard panels, asserted by `alert_templates_contract` — **met**.
- `v121.0.0` tagged after retro.

## Ordering & risks

- **121.0.1 before 121.0.2** — independent; bundled in one PR.
- **Risk — gauge vs counter semantics:** the indexer totals are gauges
  (`gauge!().set(atomic)`), not counters. Alerts use a clamp-guarded ratio
  and a restart-safe offset-delta; a process reset (gauge → 0) yields a
  negative delta, not a false page.
- **Risk — alert exprs are unvalidated by CI** (no promtool in the
  pipeline): mitigated by the metric-name contract + YAML parse check;
  expr logic is reviewed, not executed.

## References

- [[Retros/Cluster 121.0]], [[Retros/Cluster 69.0]], [[Retros/Cluster 90.0]], [[Retros/Cluster 116.0]]
- [[Remaining Work]] §1/§3; [[Open Work]]
- `docs/alerts/prometheus-rules-maidan-slo.yaml`, `docs/dashboards/maidan-operator.json`
