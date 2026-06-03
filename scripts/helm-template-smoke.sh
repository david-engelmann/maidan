#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart="${root}/helm/maidan"
stack="${root}/helm/maidan-stack"
if ! command -v helm >/dev/null 2>&1; then
  echo "helm not installed; skipping template smoke" >&2
  exit 0
fi
helm template maidan "${chart}" -f "${chart}/values.yaml" >/dev/null
helm template maidan "${chart}" -f "${chart}/values-prod.yaml" >/dev/null
helm template maidan "${chart}" -f "${chart}/values-cert-manager.yaml" >/dev/null
helm template maidan "${chart}" -f "${chart}/values-ci.yaml" >/dev/null
helm template maidan "${chart}" \
  -f "${chart}/values-prod.yaml" \
  -f "${chart}/values-cert-manager.yaml" \
  -f "${chart}/values-profile-otel.yaml" \
  -f "${chart}/values-profile-redis.yaml" >/dev/null
helm template maidan "${chart}" \
  -f "${chart}/values-prod.yaml" \
  -f "${chart}/values-profile-s3.yaml" >/dev/null
if [[ -f "${stack}/Chart.lock" ]]; then
  helm template maidan-stack "${stack}" >/dev/null
  helm template maidan-stack "${stack}" \
    --set postgresql.enabled=true \
    --set minio.enabled=true >/dev/null
  if [[ -f "${stack}/values-prod.yaml" ]]; then
    helm template maidan-stack "${stack}" -f "${stack}/values-prod.yaml" >/dev/null
  fi
fi
echo "helm template smoke OK"
