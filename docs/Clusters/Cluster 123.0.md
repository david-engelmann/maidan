# Cluster 123.0 — OTLP end-to-end smoke

**Theme:** Prove the OTLP export wiring (shipped in Cluster 89) works against a
real OpenTelemetry Collector — not just the in-process `metrics_push` unit test.
Closes the last observability sliver named after Cluster 122.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v123.0.0`**, no new
gate tag.

**Predecessor:** Cluster 89 (OTLP traces + metrics export), Cluster 122 (which
corrected the OTLP status and named the e2e smoke as the residual gap).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Compose (123.0.1)** | `otlp` profile — `otel-collector` (contrib image, debug exporter) + `maidan-otlp` (server with `OTLP_ENDPOINT`/`OTLP_METRICS=1`, short push interval). |
| **Collector (123.0.1)** | `docker/otel-collector-config.yaml` — OTLP/gRPC receiver → debug exporter (detailed) for traces + metrics. |
| **Smoke (123.0.1)** | `scripts/otlp-smoke.sh` — up, drive traffic, assert the collector logged a traces batch (incl. `http_request` span), a metrics batch, and `service.name=maidan-otlp-smoke`. |
| **CI (123.0.1)** | `otlp smoke` job (build images → run the script). |
| **Docs (123.0.2)** | Production.md OTLP verification runbook; Remaining Work §1/§3 — close the OTLP-smoke gap and correct the stale "durable job store" line (shipped in Cluster 104). |

## Non-goals

- Promoting `otlp smoke` to a required check (runs on every PR; promotion is a
  branch-protection op).
- Span/metric *content* assertions beyond presence + `service.name` + the
  `http_request` span (attribute-level golden output is brittle across SDKs).
- Consolidating the superseded `validate-prometheus-rules.sh` (noted as a
  follow-up; out of this cluster's scope).

## PR ladder (actual)

| # | Title |
|---|--------|
| 123.0.1–2 | `feat(ci): OTLP end-to-end collector smoke` (#TBD) |
| 123.0.retro | `docs(retro): Cluster 123.0 + v123.0.0 tag prep` |

## Exit criteria

- `otlp smoke` asserts a real collector receives both traces and metrics from
  the server — **met**.
- OTLP-smoke gap closed in Remaining Work; durable-reindex stale line corrected.
- `v123.0.0` tagged after retro.

## Ordering & risks

- **Assert via collector stdout, not a file.** The debug exporter logs received
  telemetry; `docker compose logs` is reachable regardless of the collector
  image's filesystem/uid (a bind-mounted file is not).
- **Timing.** Metrics push interval is set to 2s and the trace batch flushes
  ~5s; the smoke waits 12s after driving traffic.
- **Lean profile.** `localfs` + `AUTH_DISABLED` (like `maidan-a`) so the profile
  needs only postgres + collector — no MinIO.

## References

- [[Retros/Cluster 123.0]], [[Retros/Cluster 89.0]], [[Retros/Cluster 122.0]]
- `docker/otel-collector-config.yaml`, `scripts/otlp-smoke.sh`, `compose.yaml` (`otlp` profile)
- [[Production]] (OTLP verification), [[Remaining Work]] §1/§3
