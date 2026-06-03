#!/usr/bin/env bash
# Validate docs/alerts/prometheus-rules-maidan-slo.yaml (Cluster 90).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RULES="${ROOT}/docs/alerts/prometheus-rules-maidan-slo.yaml"

required_metrics=(
  http_server_request_duration_seconds
  maidan_automation_delivery_total
  maidan_outbox_pending
  maidan_outbox_oldest_pending_seconds
  maidan_outbox_quarantined
  maidan_bus_listener_ok
  maidan_indexer_last_event_age_seconds
  maidan_subscribe_replay_total
)

if [[ ! -f "${RULES}" ]]; then
  echo "missing ${RULES}" >&2
  exit 1
fi

for metric in "${required_metrics[@]}"; do
  if ! grep -q "${metric}" "${RULES}"; then
    echo "rules file missing metric reference: ${metric}" >&2
    exit 1
  fi
done

if command -v promtool >/dev/null 2>&1; then
  promtool check rules "${RULES}"
  echo "promtool: ok"
else
  echo "promtool not installed; substring checks only (ok)"
fi
