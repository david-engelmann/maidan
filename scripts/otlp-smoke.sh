#!/usr/bin/env bash
# Cluster 123 — OTLP end-to-end smoke.
#
# Brings up postgres + an OpenTelemetry Collector + a maidan-server configured
# to push OTLP traces *and* metrics (OTLP_ENDPOINT + OTLP_METRICS=1), drives a
# little HTTP traffic, then asserts the collector actually received both signals
# from our service. This proves the Cluster 89 export wiring works against a
# real collector — not just the in-process metrics_push unit test.
set -euo pipefail

cd "$(dirname "$0")/.."

PORT="${MAIDAN_OTLP_PORT:-8082}"
SERVICE="maidan-otlp-smoke"
COMPOSE=(docker compose --profile otlp)

cleanup() { "${COMPOSE[@]}" down -v >/dev/null 2>&1 || true; }
trap cleanup EXIT

"${COMPOSE[@]}" up -d

# Wait for the server to come up.
ready=
for _ in $(seq 1 60); do
  if curl -sf "http://localhost:${PORT}/health" >/tmp/otlp-health.json 2>/dev/null; then
    ready=1
    break
  fi
  sleep 2
done
if [[ -z "${ready}" ]]; then
  echo "::error::maidan-otlp /health timed out after 120s"
  "${COMPOSE[@]}" logs maidan-otlp || true
  exit 1
fi

# Drive traffic so per-request `http_request` spans + request metrics are emitted.
for _ in $(seq 1 20); do
  curl -sf "http://localhost:${PORT}/health" >/dev/null 2>&1 || true
  curl -sf "http://localhost:${PORT}/metrics" >/dev/null 2>&1 || true
done

# Metrics push interval is 2s; the trace batch processor flushes ~5s. Give margin.
sleep 12

logs="$("${COMPOSE[@]}" logs otel-collector 2>&1)"

fail=0
assert() { # <grep-pattern> <human description>
  if grep -qF -- "$1" <<<"${logs}"; then
    echo "ok: $2"
  else
    echo "::error::collector did not receive: $2 (looked for '$1')"
    fail=1
  fi
}

# service.name proves our server's telemetry reached the collector; ResourceSpans
# / ResourceMetrics prove each pipeline fired; http_request proves real per-request
# trace instrumentation is exported (not just startup spans).
assert "${SERVICE}" "resource service.name=${SERVICE}"
assert "ResourceSpans" "a traces batch"
assert "ResourceMetrics" "a metrics batch"
assert "http_request" "the per-request 'http_request' span"

if [[ "${fail}" -ne 0 ]]; then
  echo "=== otel-collector logs (tail) ==="
  tail -n 100 <<<"${logs}"
  exit 1
fi

echo "OTLP end-to-end smoke passed: traces + metrics delivered to the collector"
