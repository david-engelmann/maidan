# Cluster 121.0 retro — Observability & contract completeness

> Tag **`v121.0.0`**. First cluster of **Phase XXIV** — post-gate
> hardening after Product Ladder 102+ closed at `maidan-scale-1.0`
> (`v120.0.0`). No new gate tag.

## What shipped

- **`every_openapi_operation_is_bearer_session_or_public`** (121.0.1):
  a classification guard that walks every operation in `ApiDoc::openapi()`
  and requires each to be bearer-authenticated (and thus in
  `contracts/http-capability-map.json`, enforced by the pre-existing
  bidirectional match), session-cookie-gated (`SESSION_OPERATIONS`), or
  explicitly public (`PUBLIC_OPERATIONS` — health/metrics/spec/discovery/
  OIDC handshake). A new route that ships with neither auth nor a
  capability mapping now fails CI. Closes the OpenAPI-wide capability-map
  gap deferred since Cluster 69.
- **Scale-out SLO coverage** (121.0.2) for the Cluster 116 batched-embed
  indexer gauges:
  - recording rule `maidan_slo:indexer_queue_saturation` (clamp-guarded
    `queue_depth / queue_capacity`);
  - `MaidanIndexerQueueSaturated` — queue >80% full for 5m (embed
    backpressure; live writes block on the bounded queue);
  - `MaidanIndexerEmbedFailures` — offset-delta on the monotonic
    `embed_failed_total` gauge over 15m;
  - two operator-dashboard panels (queue depth vs capacity; embed failures);
  - `alert_templates_contract` `expected[]` now asserts the three metrics.
- **`Remaining Work.md` §1/§3** corrected: capability-map gap closed;
  SLO dashboards/alerts noted as extended to scale-out metrics.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Cluster 89 | OTLP export wiring | The other half of "Observability (76)"; dashboards/alerts done, exporter not. |
| Post-gate | promtool/`amtool` rule unit tests in CI | No Prometheus toolchain in the pipeline; mitigated by name contract + YAML parse. |
| Doc debt | `Open Work.md` stale tail (a v76/cluster-78 historical block) | Pre-existing; out of this cluster's two-item scope — flagged for a doc sweep. |

## Surprises

- **The capability-map gap was already 80% closed.** The bidirectional
  bearer↔map exact-match shipped earlier; the only missing piece was the
  *negative* check — "is there an op that's neither auth'd nor mapped?".
  One test, not a subsystem.
- **`/auth/session` + `/auth/session/mint` are neither bearer nor public.**
  They are gated by the `session_auth` cookie layer in `app.rs`, so they
  carry no bearer capability but must not be treated as public — hence the
  separate `SESSION_OPERATIONS` class. The honesty check applies only to
  these (documented ops); `/metrics`, `/openapi.json` etc. are intentionally
  absent from the ApiDoc and exempt.
- **The indexer "totals" are gauges, not counters.** `refresh_runtime_gauges`
  emits `embed_failed_total` via `gauge!().set(atomic)`. `rate()`/`increase()`
  would silently misread it; the alert uses an offset-delta that is
  restart-safe (a reset to 0 → negative delta → no false page).

## Decisions

- **Classify, don't just match.** The bidirectional match proves bearer ops
  and the map agree; the classification guard proves no op *escapes* the
  taxonomy. The two together make "every route is accounted for" a CI
  invariant.
- **Gauge-correct alert math.** Ratio for saturation, offset-delta for the
  monotonic failure gauge — never `rate()` on a `set()` gauge.
- **Extend the Cluster 90 artifacts, don't fork them.** New rules/panels
  live in the existing `prometheus-rules-maidan-slo.yaml` +
  `maidan-operator.json`; the contract test grows by three names.

## Capability table extension

| Capability | Where |
|------------|-------|
| Every OpenAPI op classified (bearer/session/public) in CI | `crates/maidan-server/tests/http_openapi_capability_map_contract.rs` |
| Indexer queue-saturation recording rule + backpressure/embed-failure alerts | `docs/alerts/prometheus-rules-maidan-slo.yaml` |
| Operator dashboard panels for indexer queue depth + embed failures | `docs/dashboards/maidan-operator.json` |

## Risks identified + still open

- **Alert exprs aren't executed in CI.** Only metric *names* are contract-
  checked; PromQL correctness is reviewed, not run. A promtool unit-test job
  would close this (deferred).
- **OTLP export still open** (Cluster 89). The SLO surface is complete; the
  push-based export path is not.

## Forward look

Post-gate hardening continues opportunistically from [[Remaining Work]] and
[[Open Work]]; no ladder is defined past 120. Candidate next gaps: OTLP
exporter wiring (89), promtool rule unit tests, and the `Open Work.md`
stale-tail doc sweep.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
