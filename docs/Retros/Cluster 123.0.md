# Cluster 123.0 retro — OTLP end-to-end smoke

> Tag **`v123.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **`otlp` compose profile** (123.0.1): an `otel-collector`
  (`otel/opentelemetry-collector-contrib:0.154.0`, debug exporter) plus a
  `maidan-otlp` server configured with `OTLP_ENDPOINT` + `OTLP_METRICS=1` and a
  2s push interval. `localfs` + `AUTH_DISABLED` keep the profile to just
  postgres + collector — no MinIO.
- **`docker/otel-collector-config.yaml`** (123.0.1): OTLP/gRPC receiver → debug
  exporter (detailed) for both the traces and metrics pipelines, logged to stdout.
- **`scripts/otlp-smoke.sh`** (123.0.1): brings the profile up, drives HTTP
  traffic, then asserts the collector received a traces batch (incl. the
  per-request `http_request` span), a metrics batch, and resource
  `service.name=maidan-otlp-smoke`. Verified locally — all four assertions pass.
- **`otlp smoke` CI job** (123.0.1, non-required): builds the images, runs the
  script, dumps collector + server logs on failure.
- **Docs** (123.0.2): a Production.md OTLP verification runbook; Remaining Work
  §1/§3 — close the OTLP-smoke gap and correct the stale "durable job store"
  line (durable reindex jobs shipped in Cluster 104).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Follow-up | Promote `otlp smoke` to a required check | Runs on every PR; promotion is a branch-protection op. |
| Follow-up | Consolidate the superseded `validate-prometheus-rules.sh` | Substring-only local check; CI uses `check-alert-rules.sh` (v122). Out of this cluster's scope. |
| Out of scope | Attribute-level golden assertions on spans/metrics | Brittle across SDK/collector versions; presence + `service.name` + the `http_request` span is the durable contract. |

## Surprises

- **Assert via stdout, not a file.** The first instinct — have the collector
  write received telemetry to a bind-mounted file and grep it — runs into
  container uid/filesystem friction. The debug exporter logs everything to
  stdout, and `docker compose logs` is always reachable from the host. Simpler
  and more portable.
- **A second phantom gap.** While writing the verification runbook, the
  Production doc revealed durable reindex jobs shipped in Cluster 104 — yet
  Remaining Work §1 still listed "durable job store" as open. Two stale backlog
  lines (this and OTLP) corrected by reading the code/docs, not the backlog.
- **Both signals flow from one HTTP request.** The `http_request` info-span
  (from the request-id middleware) exports through the `tracing_opentelemetry`
  layer, and the same request increments the `http.server.request_total`
  counter that the OTLP metrics fanout pushes — so a few `curl`s exercise both
  pipelines at once.

## Decisions

- **Lean, isolated profile.** `localfs` + `AUTH_DISABLED` (like `maidan-a`) so
  the smoke needs only postgres + collector; no MinIO, no tokens.
- **Short push interval for a fast smoke.** `OTLP_METRICS_INTERVAL_SECS=2` (vs
  the 15s default) so the metrics batch lands within the 12s wait.
- **Prove it locally first.** The whole deliverable is the smoke; it was run
  end-to-end against a real collector before the PR, so CI confirms rather than
  discovers.

## Capability table extension

| Capability | Where |
|------------|-------|
| OTLP traces + metrics asserted end-to-end against a real collector | `compose.yaml` (`otlp` profile), `docker/otel-collector-config.yaml`, `scripts/otlp-smoke.sh`, `.github/workflows/ci.yml` (`otlp smoke`) |

## Risks identified + still open

- **`otlp smoke` is non-required.** Runs on every PR but doesn't block until
  promoted in branch protection.
- **Timing-sensitive.** The assertion depends on the 2s push interval and trace
  batch flush landing inside the 12s wait; generous, but a very slow runner
  could flake. The failure path dumps collector + server logs to diagnose.

## Forward look

Observability is now end-to-end: export wiring (89), SLO rules + dashboards (90,
121), promtool execution (122), and collector delivery (123). Candidate next:
event-bus delivery dedup (the oldest standing risk), or the
`validate-prometheus-rules.sh` consolidation.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
