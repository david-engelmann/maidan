#!/usr/bin/env bash
# Maidan backup (Cluster 260, Program D — disaster recovery).
#
# Captures the two pieces of durable state:
#   1. Postgres  — the system of record (all workspaces/messages/events/audit/
#      tokens...). Dumped with pg_dump in the custom format (-Fc), which restore.sh
#      feeds to pg_restore.
#   2. Artifacts — content-addressed blobs. For ARTIFACT_BACKEND=localfs the root
#      is tarred; for s3 the bucket is the durable store (enable versioning /
#      cross-region replication there) and is NOT copied here.
#
# NOT backed up (restore these out of band): secrets/config — DATABASE_URL,
# MAIDAN_SESSION_SECRET, FEDERATION_ENCRYPTION_KEY (+ FEDERATION_DECRYPT_KEYS),
# SMTP/OIDC creds. They live in your secret manager, not in the data backup.
#
# Usage:
#   DATABASE_URL=postgres://…  scripts/backup.sh [BACKUP_DIR]
#   ARTIFACT_LOCALFS_ROOT=/var/lib/maidan/artifacts  DATABASE_URL=…  scripts/backup.sh
#
# BACKUP_DIR defaults to ./backups/<UTC-timestamp>. Prints the directory it wrote.
set -euo pipefail

: "${DATABASE_URL:?set DATABASE_URL to the Postgres connection string}"

ts="$(date -u +%Y%m%dT%H%M%SZ)"
out="${1:-${BACKUP_DIR:-backups/$ts}}"
mkdir -p "$out"

echo "backup: dumping Postgres → $out/postgres.dump"
pg_dump --format=custom --no-owner --no-privileges --file "$out/postgres.dump" "$DATABASE_URL"

if [[ "${ARTIFACT_BACKEND:-localfs}" == "localfs" ]]; then
  root="${ARTIFACT_LOCALFS_ROOT:-}"
  if [[ -n "$root" && -d "$root" ]]; then
    echo "backup: archiving artifacts ($root) → $out/artifacts.tar.gz"
    tar -czf "$out/artifacts.tar.gz" -C "$root" .
  else
    echo "backup: ARTIFACT_LOCALFS_ROOT unset or missing — skipping artifact archive" >&2
  fi
else
  echo "backup: ARTIFACT_BACKEND=$ARTIFACT_BACKEND — object store is the durable copy (enable bucket versioning); not archived here" >&2
fi

# A small manifest makes restore.sh (and humans) sanity-check what this is.
cat > "$out/MANIFEST.txt" <<MANIFEST
maidan-backup
created_utc=$ts
postgres_dump=postgres.dump
artifact_backend=${ARTIFACT_BACKEND:-localfs}
artifact_archive=$([[ -f "$out/artifacts.tar.gz" ]] && echo artifacts.tar.gz || echo none)
MANIFEST

echo "backup: complete → $out"
