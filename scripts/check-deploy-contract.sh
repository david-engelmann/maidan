#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if rg -n 'quay\.io/minio/(minio|mc):latest' \
  compose.yaml compose.dev.yaml k8s helm; then
  echo "MinIO deployment images must use immutable release tags, not :latest" >&2
  exit 1
fi

# Chainguard publishes MinIO only as :latest, so every deploy path pins it by
# digest, and they pin the same one: a bump in compose that leaves the stack
# chart or k8s on the old build is a split nobody chose. The chart keeps the
# repository and digest on separate lines of its values.
stack_values=helm/maidan-stack/values.yaml
for name in minio minio-client; do
  chart_pin="$(awk -v repo="cgr.dev/chainguard/${name}" '
    $1 == "repository:" { cur = ($2 == repo) }
    cur && $1 == "digest:" { print repo "@" $2; cur = 0 }
  ' "${stack_values}")"
  [[ -n "${chart_pin}" ]] || {
    echo "${stack_values} pins no digest for cgr.dev/chainguard/${name}; if it moved, update this script" >&2
    exit 1
  }
  # Any reference to the image that is not a full digest (a tag, or none) is
  # refused before the pins are compared, or it would silently drop out of them.
  loose="$(rg -oP --no-filename "cgr\.dev/chainguard/${name}(?![-\w])[^\"'\s]*" \
    compose.yaml compose.dev.yaml k8s | rg -v "^cgr\.dev/chainguard/${name}@sha256:[0-9a-f]{64}$" || true)"
  if [[ -n "${loose}" ]]; then
    echo "cgr.dev/chainguard/${name} must be referenced by a full sha256 digest; found:" >&2
    echo "${loose}" >&2
    exit 1
  fi
  [[ "${chart_pin}" =~ ^cgr\.dev/chainguard/${name}@sha256:[0-9a-f]{64}$ ]] || {
    echo "${stack_values}: ${chart_pin} is not a full sha256 digest" >&2
    exit 1
  }
  pins="$(
    {
      rg -o --no-filename "cgr\.dev/chainguard/${name}@sha256:[0-9a-f]{64}" \
        compose.yaml compose.dev.yaml k8s || true
      echo "${chart_pin}"
    } | sort -u
  )"
  if [[ "$(grep -c . <<<"${pins}")" -ne 1 ]]; then
    echo "cgr.dev/chainguard/${name} must be pinned to one digest across compose.yaml, compose.dev.yaml, k8s/ and ${stack_values}; found:" >&2
    echo "${pins}" >&2
    exit 1
  fi
done

echo "deployment contract OK"
