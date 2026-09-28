#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart="${root}/helm/maidan"
stack="${root}/helm/maidan-stack"
if ! command -v helm >/dev/null 2>&1; then
  echo "helm not installed; skipping template smoke" >&2
  exit 0
fi
# Every render names a content KEK; the chart refuses without one.
kek="$(openssl rand -hex 32)"
if helm template maidan "${chart}" -f "${chart}/values.yaml" >/dev/null 2>&1; then
  echo "helm template rendered without contentKek; it must refuse" >&2
  exit 1
fi
helm template maidan "${chart}" --set existingSecret=maidan-secrets \
  | grep -q "key: MAIDAN_CONTENT_KEK" || {
  echo "the deployment must read MAIDAN_CONTENT_KEK from the existing Secret" >&2
  exit 1
}
helm template maidan "${chart}" --set contentKek="${kek}" \
  | grep -q "MAIDAN_CONTENT_KEK: \"${kek}\"" || {
  echo "the rendered Secret must carry contentKek" >&2
  exit 1
}
helm template maidan "${chart}" -f "${chart}/values.yaml" --set contentKek="${kek}" >/dev/null
helm template maidan "${chart}" -f "${chart}/values-prod.yaml" --set contentKek="${kek}" >/dev/null
helm template maidan "${chart}" -f "${chart}/values-cert-manager.yaml" --set contentKek="${kek}" >/dev/null
helm template maidan "${chart}" -f "${chart}/values-ci.yaml" --set contentKek="${kek}" >/dev/null
helm template maidan "${chart}" \
  -f "${chart}/values-prod.yaml" \
  -f "${chart}/values-cert-manager.yaml" \
  -f "${chart}/values-profile-otel.yaml" \
  -f "${chart}/values-profile-redis.yaml" --set contentKek="${kek}" >/dev/null
helm template maidan "${chart}" \
  -f "${chart}/values-prod.yaml" \
  -f "${chart}/values-profile-s3.yaml" --set contentKek="${kek}" >/dev/null
if [[ -f "${stack}/Chart.lock" ]]; then
  if helm template maidan-stack "${stack}" >/dev/null 2>&1; then
    echo "the stack rendered without maidan.contentKek; it must refuse" >&2
    exit 1
  fi
  helm template maidan-stack "${stack}" --set maidan.contentKek="${kek}" >/dev/null
  helm template maidan-stack "${stack}" \
    --set postgresql.enabled=true \
    --set minio.enabled=true --set maidan.contentKek="${kek}" >/dev/null
  if [[ -f "${stack}/values-prod.yaml" ]]; then
    helm template maidan-stack "${stack}" -f "${stack}/values-prod.yaml" \
      --set maidan.contentKek="${kek}" >/dev/null
  fi
fi
echo "helm template smoke OK"
