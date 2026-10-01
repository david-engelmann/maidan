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
# refuses <message fragment> <helm template args…>: the render must fail, and
# say why.
refuses() {
  local want="$1" out
  shift
  if out="$(helm template "$@" 2>&1)"; then
    echo "helm template $* rendered; it must refuse (${want})" >&2
    exit 1
  fi
  grep -qF -- "${want}" <<<"${out}" || {
    echo "helm template $* refused without saying \"${want}\":" >&2
    echo "${out}" >&2
    exit 1
  }
}
# The stack renders its vendored copy of the chart, not helm/maidan, so a
# chart change that is not repackaged never reaches a stack install
# (`helm package helm/maidan -d helm/maidan-stack/charts`).
vendored="$(mktemp -d)"
trap 'rm -rf "${vendored}"' EXIT
tar -xzf "${stack}/charts/maidan-0.1.0.tgz" -C "${vendored}"
diff -r --exclude Chart.yaml "${vendored}/maidan" "${chart}" >&2 || {
  echo "helm/maidan-stack/charts/maidan-0.1.0.tgz is stale: helm package helm/maidan -d helm/maidan-stack/charts" >&2
  exit 1
}
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
helm template maidan "${chart}" -f "${chart}/values-ci.yaml" --set contentKek="${kek}" >/dev/null
# A production render needs real credentials: a database URL that is not the
# chart's development default, or an existing Secret.
db="postgres://maidan:smoke-password@db.internal:5432/maidan"
prod=(maidan "${chart}" -f "${chart}/values-prod.yaml")
helm template "${prod[@]}" --set secrets.DATABASE_URL="${db}" --set contentKek="${kek}" >/dev/null
helm template "${prod[@]}" --set existingSecret=maidan-secrets >/dev/null
helm template "${prod[@]}" -f "${chart}/values-cert-manager.yaml" \
  --set secrets.DATABASE_URL="${db}" --set contentKek="${kek}" >/dev/null
# A production render names a release image, never the local development one.
refuses "maidan-server, the local development image" \
  maidan "${chart}" -f "${chart}/values-cert-manager.yaml" --set existingSecret=maidan-secrets
refuses "set image.tag to a release" \
  "${prod[@]}" --set image.tag= --set existingSecret=maidan-secrets
refuses "image.tag \"dev\" is not a release" \
  "${prod[@]}" --set image.tag=dev --set existingSecret=maidan-secrets
refuses "image.digest still holds the placeholder CHANGE_ME" \
  "${prod[@]}" --set image.digest=CHANGE_ME --set existingSecret=maidan-secrets
refuses "image.tag still holds the placeholder CHANGE_ME" \
  "${prod[@]}" --set image.digest= --set image.tag=CHANGE_ME --set existingSecret=maidan-secrets
refuses "image.tag still holds the placeholder CHANGE_ME" \
  maidan "${chart}" --set image.tag=CHANGE_ME --set contentKek="${kek}"
helm template "${prod[@]}" --set image.tag= \
  --set image.digest=sha256:0000000000000000000000000000000000000000000000000000000000000000 \
  --set existingSecret=maidan-secrets | grep -q 'image: "ghcr.io/david-engelmann/maidan-server@sha256:0000' || {
  echo "a digest must stand in for the tag in a production render" >&2
  exit 1
}
refuses "secrets.DATABASE_URL is the chart's development default" \
  "${prod[@]}" --set contentKek="${kek}"
refuses "a production install needs its database" \
  "${prod[@]}" --set secrets.DATABASE_URL= --set contentKek="${kek}"
refuses "secrets.S3_SECRET_ACCESS_KEY is empty" \
  "${prod[@]}" --set secrets.DATABASE_URL="${db}" --set secrets.S3_SECRET_ACCESS_KEY= --set contentKek="${kek}"
# A CHANGE_ME placeholder fails every render, dev included.
refuses "secrets.DATABASE_URL still holds the placeholder CHANGE_ME" \
  maidan "${chart}" --set secrets.DATABASE_URL="postgres://maidan:CHANGE_ME@db:5432/maidan" --set contentKek="${kek}"
refuses "config.S3_ENDPOINT still holds the placeholder CHANGE_ME" \
  maidan "${chart}" --set config.S3_ENDPOINT=CHANGE_ME --set contentKek="${kek}"
refuses "contentKek still holds the placeholder CHANGE_ME" \
  maidan "${chart}" --set contentKek=CHANGE_ME
helm template "${prod[@]}" \
  -f "${chart}/values-cert-manager.yaml" \
  -f "${chart}/values-profile-otel.yaml" \
  -f "${chart}/values-profile-redis.yaml" \
  --set secrets.DATABASE_URL="${db}" --set contentKek="${kek}" >/dev/null
refuses "secrets.S3_ACCESS_KEY_ID still holds the placeholder CHANGE_ME" \
  "${prod[@]}" -f "${chart}/values-profile-s3.yaml" \
  --set secrets.DATABASE_URL="${db}" --set contentKek="${kek}"
helm template "${prod[@]}" -f "${chart}/values-profile-s3.yaml" \
  --set secrets.DATABASE_URL="${db}" --set secrets.S3_ACCESS_KEY_ID=smoke \
  --set secrets.S3_SECRET_ACCESS_KEY=smoke --set contentKek="${kek}" >/dev/null
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
    stack_prod=(maidan-stack "${stack}" -f "${stack}/values-prod.yaml")
    rendered="$(helm template "${stack_prod[@]}" --set maidan.secrets.DATABASE_URL="${db}" \
      --set maidan.secrets.S3_SECRET_ACCESS_KEY=smoke --set maidan.contentKek="${kek}")"
    grep -q 'image: "ghcr.io/david-engelmann/maidan-server:v' <<<"${rendered}" || {
      echo "the stack's prod values must render the release image" >&2
      exit 1
    }
    grep -q 'S3_ENDPOINT: "http://maidan-stack-minio:9000"' <<<"${rendered}" || {
      echo "the stack's prod S3_ENDPOINT must name the release's MinIO Service" >&2
      exit 1
    }
    helm template "${stack_prod[@]}" --set maidan.existingSecret=maidan-secrets >/dev/null
    refuses "set image.tag to a release" \
      "${stack_prod[@]}" --set maidan.image.tag= --set maidan.existingSecret=maidan-secrets
    refuses "a production install needs its database" \
      "${stack_prod[@]}" --set maidan.contentKek="${kek}"
    refuses "secrets.S3_SECRET_ACCESS_KEY is empty" \
      "${stack_prod[@]}" --set maidan.secrets.DATABASE_URL="${db}" --set maidan.contentKek="${kek}"
  fi
fi
echo "helm template smoke OK"
