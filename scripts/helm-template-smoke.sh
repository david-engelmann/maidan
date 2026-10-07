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
# Secrets reach the server as files: each key is a file under
# /run/secrets/maidan with a <KEY>_FILE pointing at it, and no Secret is passed
# through envFrom.
files="$(helm template maidan "${chart}" --set contentKek="${kek}")"
grep -q "name: MAIDAN_CONTENT_KEK_FILE" <<<"${files}" && grep -q "name: DATABASE_URL_FILE" <<<"${files}" || {
  echo "the deployment must name each secret file in <KEY>_FILE" >&2
  exit 1
}
if grep -q -- "- secretRef:" <<<"${files}"; then
  echo "with secretFiles on, no Secret may reach the server through envFrom" >&2
  exit 1
fi
helm template maidan "${chart}" --set contentKek="${kek}" --set secretFiles.enabled=false \
  | grep -q -- "- secretRef:" || {
  echo "with secretFiles off, the Secret must reach the server through envFrom" >&2
  exit 1
}
refuses "MAIDAN_LOG in Secret maidan-maidan-secrets cannot be read from a file" \
  maidan "${chart}" --set contentKek="${kek}" --set secrets.MAIDAN_LOG=info
refuses "S3_ACCESS_KEY_ID is mounted from both a and b" \
  maidan "${chart}" --set contentKek="${kek}" \
  --set-json 'secretFiles.extra=[{"name":"a","keys":["S3_ACCESS_KEY_ID"]},{"name":"b","keys":["S3_ACCESS_KEY_ID"]}]'
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
  # has <rendered> <fixed text, lines included> <why>: the render contains it.
  has() {
    [[ "$1" == *"$2"* ]] || {
      echo "$3 (expected: $2)" >&2
      exit 1
    }
  }
  if helm template maidan-stack "${stack}" >/dev/null 2>&1; then
    echo "the stack rendered without maidan.contentKek; it must refuse" >&2
    exit 1
  fi
  rendered="$(helm template maidan-stack "${stack}" --set maidan.contentKek="${kek}")"
  if grep -qE '^kind: (StatefulSet|Job)$' <<<"${rendered}"; then
    echo "the stack rendered a store with postgresql and minio off" >&2
    exit 1
  fi
  # The server reads the stack's MinIO settings after its own ConfigMap, so
  # they win; optional, so a stack running neither store still starts.
  has "${rendered}" "$(printf '%s\n' \
    "                name: maidan-stack-maidan-config" \
    "            - configMapRef:" \
    "                name: 'maidan-stack-datastores'" \
    "                optional: true")" "the server must read maidan-stack-datastores after its own ConfigMap"
  datastores='[{"name":"{{ .Release.Name }}-datastores","keys":["DATABASE_URL","S3_ACCESS_KEY_ID","S3_SECRET_ACCESS_KEY"]}]'
  stores_unwired=(maidan-stack "${stack}" --set postgresql.enabled=true --set minio.enabled=true
    --set maidan.contentKek="${kek}")
  stores=("${stores_unwired[@]}" --set-json "maidan.secretFiles.extra=${datastores}")
  rendered="$(helm template "${stores[@]}")"
  has "${rendered}" 'DATABASE_URL: "postgres://maidan:maidan@maidan-stack-postgresql:5432/maidan"' \
    "with postgresql on, the server's DATABASE_URL must name the bundled database"
  has "${rendered}" "$(printf '%s\n' \
    "                  name: maidan-stack-datastores" \
    "                  items:" \
    "                    - key: DATABASE_URL")" "the bundled database's URL must reach the server as a file"
  has "${rendered}" 'ARTIFACT_BACKEND: "s3"' "with minio on, the server must store artifacts in S3"
  has "${rendered}" 'S3_ENDPOINT: "http://maidan-stack-minio:9000"' \
    "with minio on, S3_ENDPOINT must name the release's MinIO Service"
  has "${rendered}" 'S3_BUCKET: "maidan-artifacts"' "S3_BUCKET must be the first of minio.defaultBuckets"
  has "${rendered}" 'image: "maidan-postgres:dev"' "the dev Postgres is the locally built pgvector image"
  has "${rendered}" 'image: "cgr.dev/chainguard/minio@sha256:' "MinIO must be Chainguard's, by digest"
  has "${rendered}" '- "local/maidan-artifacts"' "the bucket Job must create minio.defaultBuckets"
  has "${rendered}" '"pg_isready", "-h", "127.0.0.1"' "the Postgres probes must use pg_isready over TCP"
  # The MinIO annotation is sha256 of Secret key rollout-nonce. It must not be
  # a hash of the root user or password: a StatefulSet reader could test guesses
  # against that. helm template has no live Secret, so every offline render
  # uses the fixed nonce "stable", including when the password changes.
  RENDERED="${rendered}" python3 -c '
import hashlib, os, sys
rendered = os.environ["RENDERED"]
def one(prefix):
    found = [line.split(": ", 1)[1].strip().strip(chr(34))
             for line in rendered.splitlines()
             if line.startswith(prefix)]
    if len(found) != 1 or not found[0]:
        sys.exit("expected one %r, found %r" % (prefix, found))
    return found[0]
nonce = one("  rollout-nonce: ")
digest = one("        checksum/rollout-nonce: ")
want = hashlib.sha256(nonce.encode()).hexdigest()
if digest != want:
    sys.exit("checksum/rollout-nonce is not sha256 of rollout-nonce")
user, password = "minio", "minioadmin"
for material in (password, user + ":" + password, user + password):
    if hashlib.sha256(material.encode()).hexdigest() in rendered:
        sys.exit("render publishes a hash of the MinIO root credentials")
if "checksum/credentials" in rendered:
    sys.exit("render still has checksum/credentials")
docs = rendered.split("\n---\n")
sts = [d for d in docs if "kind: StatefulSet" in d and "checksum/rollout-nonce" in d]
if len(sts) != 1:
    sys.exit("expected one MinIO StatefulSet, found %d" % len(sts))
if password in sts[0]:
    sys.exit("MinIO StatefulSet manifest contains the root password")
'
  other="$(helm template "${stores[@]}")"
  nonce_a="$(sed -n "s/^  rollout-nonce: \"\\([A-Za-z0-9]*\\)\"$/\\1/p" <<<"${rendered}")"
  nonce_b="$(sed -n "s/^  rollout-nonce: \"\\([A-Za-z0-9]*\\)\"$/\\1/p" <<<"${other}")"
  if [[ "${nonce_a}" != "stable" || "${nonce_b}" != "stable" ]]; then
    echo "helm template rollout-nonce must be the stable offline value, not a password hash" >&2
    exit 1
  fi
  changed="$(helm template "${stores[@]}" --set minio.auth.rootPassword=not-the-default)"
  nonce_c="$(sed -n "s/^  rollout-nonce: \"\\([A-Za-z0-9]*\\)\"$/\\1/p" <<<"${changed}")"
  if [[ "${nonce_c}" != "stable" ]]; then
    echo "offline rollout-nonce must not change with the MinIO password" >&2
    exit 1
  fi
  pw_hash="$(printf '%s' 'not-the-default' | sha256sum | awk '{print $1}')"
  if grep -q "${pw_hash}" <<<"${changed}"; then
    echo "render publishes a hash of the MinIO root password" >&2
    exit 1
  fi
  # Persistence preflight needs a live StatefulSet. Offline renders still have
  # to succeed when enabled, size, or storageClass differ from the defaults.
  helm template "${stores[@]}" --set minio.persistence.enabled=false >/dev/null
  helm template "${stores[@]}" --set minio.persistence.size=20Gi \
    --set minio.persistence.storageClass=smoke >/dev/null
  # Credentials are escaped into the URL rather than breaking it.
  has "$(helm template "${stores[@]}" --set 'postgresql.auth.password=p@ss w/rd:+%')" \
    'postgres://maidan:p%40ss%20w%2Frd%3A%2B%25@maidan-stack-postgresql:5432/maidan' \
    "DATABASE_URL must percent-encode the password"
  helm template maidan-stack "${stack}" -f "${stack}/values-ci.yaml" --set maidan.contentKek="${kek}" >/dev/null
  refuses "postgresql.auth.password still holds the placeholder CHANGE_ME" \
    "${stores[@]}" --set postgresql.auth.password=CHANGE_ME
  refuses "minio.auth.rootPassword is shorter than 8 characters" \
    "${stores[@]}" --set minio.auth.rootPassword=short
  refuses "which MC_HOST_local cannot carry" \
    "${stores[@]}" --set minio.auth.rootPassword=has:colon
  refuses "minio.defaultBuckets is empty" \
    "${stores[@]}" --set 'minio.defaultBuckets=\ \,'
  refuses "maidan.secretFiles.extra must list Secret maidan-stack-datastores with keys DATABASE_URL, S3_ACCESS_KEY_ID, S3_SECRET_ACCESS_KEY" \
    "${stores_unwired[@]}"
  refuses "maidan.extraEnvFrom must keep the configMapRef to maidan-stack-datastores" \
    "${stores[@]}" --set-json 'maidan.extraEnvFrom=[]'
  refuses "with maidan.secretFiles.enabled false, maidan.extraEnvFrom must hold a secretRef to maidan-stack-datastores" \
    "${stores_unwired[@]}" --set maidan.secretFiles.enabled=false
  helm template "${stores_unwired[@]}" --set maidan.secretFiles.enabled=false \
    --set-json 'maidan.extraEnvFrom=[{"configMapRef":{"name":"{{ .Release.Name }}-datastores"}},{"secretRef":{"name":"{{ .Release.Name }}-datastores"}}]' >/dev/null
  if [[ -f "${stack}/values-prod.yaml" ]]; then
    stack_prod=(maidan-stack "${stack}" -f "${stack}/values-prod.yaml"
      --set postgresql.auth.password=smoke-pg-password
      --set minio.auth.rootPassword=smoke-minio-password)
    rendered="$(helm template "${stack_prod[@]}" --set maidan.contentKek="${kek}")"
    has "${rendered}" 'image: "ghcr.io/david-engelmann/maidan-server:v' \
      "the stack's prod values must render the release image"
    has "${rendered}" 'image: "ghcr.io/david-engelmann/maidan-postgres:v' \
      "the stack's prod values must render the release Postgres image"
    has "${rendered}" 'DATABASE_URL: "postgres://maidan:smoke-pg-password@maidan-stack-postgresql:5432/maidan"' \
      "the stack's prod DATABASE_URL must name the bundled database"
    has "${rendered}" 'S3_ENDPOINT: "http://maidan-stack-minio:9000"' \
      "the stack's prod S3_ENDPOINT must name the release's MinIO Service"
    helm template "${stack_prod[@]}" --set maidan.existingSecret=maidan-secrets >/dev/null
    refuses "set image.tag to a release" \
      "${stack_prod[@]}" --set maidan.image.tag= --set maidan.existingSecret=maidan-secrets
    stack_prod_nopw=(maidan-stack "${stack}" -f "${stack}/values-prod.yaml" --set maidan.contentKek="${kek}")
    refuses "postgresql.auth.password is empty" "${stack_prod_nopw[@]}"
    refuses "minio.auth.rootPassword is shorter than 8 characters" \
      "${stack_prod_nopw[@]}" --set postgresql.auth.password=smoke-pg-password
    refuses "postgresql.auth.password is the chart's development default" \
      "${stack_prod[@]}" --set maidan.contentKek="${kek}" --set postgresql.auth.password=maidan
    refuses "minio.auth.rootPassword is the chart's development default" \
      "${stack_prod[@]}" --set maidan.contentKek="${kek}" --set minio.auth.rootPassword=minioadmin
    refuses "postgresql.image.tag \"\" is not a release" \
      "${stack_prod[@]}" --set maidan.contentKek="${kek}" --set postgresql.image.tag=
    refuses "postgresql.image.repository is maidan-postgres, the local development image" \
      "${stack_prod[@]}" --set maidan.contentKek="${kek}" --set postgresql.image.repository=maidan-postgres
    refuses "minio.image.tag \"\" is not a release" \
      "${stack_prod[@]}" --set maidan.contentKek="${kek}" --set minio.image.digest=
    helm template "${stack_prod[@]}" --set maidan.contentKek="${kek}" \
      --set postgresql.image.tag= --set postgresql.image.digest=sha256:0000000000000000000000000000000000000000000000000000000000000000 \
      | grep -q 'image: "ghcr.io/david-engelmann/maidan-postgres@sha256:0000' || {
      echo "a digest must stand in for the Postgres tag in a production render" >&2
      exit 1
    }
    # Without the bundled database, the server's own DATABASE_URL is checked
    # here: the maidan chart leaves it to whoever sets extraEnvFrom.
    # Without the bundled Postgres, the datastores Secret holds only MinIO's keys.
    s3_only='[{"name":"{{ .Release.Name }}-datastores","keys":["S3_ACCESS_KEY_ID","S3_SECRET_ACCESS_KEY"]}]'
    refuses "maidan.secrets.DATABASE_URL is the maidan chart's development default" \
      "${stack_prod[@]}" --set maidan.contentKek="${kek}" --set postgresql.enabled=false \
      --set-json "maidan.secretFiles.extra=${s3_only}"
    helm template "${stack_prod[@]}" --set maidan.contentKek="${kek}" --set postgresql.enabled=false \
      --set-json "maidan.secretFiles.extra=${s3_only}" --set maidan.secrets.DATABASE_URL="${db}" >/dev/null
  fi
fi
echo "helm template smoke OK"
