#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart="${root}/helm/maidan"
if ! command -v helm >/dev/null 2>&1; then
  echo "helm not installed; skipping template smoke" >&2
  exit 0
fi
helm template maidan "${chart}" -f "${chart}/values.yaml" >/dev/null
helm template maidan "${chart}" -f "${chart}/values-prod.yaml" >/dev/null
echo "helm template smoke OK"
