#!/usr/bin/env bash
# Maidan backup.
#
# Captures the two pieces of durable state:
#   1. The database — the system of record (all workspaces/messages/events/
#      audit/tokens...). Postgres is dumped with pg_dump in the custom format
#      (-Fc), which restore.sh feeds to pg_restore. SQLite is copied with
#      `VACUUM INTO`, which writes a consistent, compacted snapshot while the
#      server keeps running; copying the file itself would miss what is still
#      in the -wal file, or catch a page mid-write.
#   2. Artifacts — content-addressed blobs. For ARTIFACT_BACKEND=localfs the root
#      is tarred; for s3 the bucket is the durable store (enable versioning /
#      cross-region replication there) and is NOT copied here.
#
# NOT backed up (restore these out of band): secrets/config — DATABASE_URL,
# MAIDAN_SESSION_SECRET, FEDERATION_ENCRYPTION_KEY (+ FEDERATION_DECRYPT_KEYS),
# MAIDAN_CONTENT_KEK (+ _PREVIOUS), SMTP/OIDC creds. Keeping the KEK out of the
# dump matters: a dump taken before a withdrawal plus the KEK recovers the words.
# They live in your secret manager, not in the data backup.
#
# Usage:
#   DATABASE_URL=postgres://…  scripts/backup.sh [BACKUP_DIR]
#   DATABASE_URL=sqlite:///data/maidan.db  scripts/backup.sh [BACKUP_DIR]   # needs the sqlite3 CLI
#   ARTIFACT_LOCALFS_ROOT=/var/lib/maidan/artifacts  DATABASE_URL=…  scripts/backup.sh
#
# BACKUP_DIR defaults to./backups/<UTC-timestamp>. Prints the directory it wrote.
set -euo pipefail

: "${DATABASE_URL:?set DATABASE_URL to the database URL (postgres:// or sqlite:)}"

ts="$(date -u +%Y%m%dT%H%M%SZ)"
out="${1:-${BACKUP_DIR:-backups/$ts}}"
mkdir -p "$out"

case "$DATABASE_URL" in
  sqlite:*)
    database=sqlite
    # sqlite:///abs/path, sqlite://rel/path and sqlite:rel/path, minus ?options.
    db="${DATABASE_URL#sqlite:}"
    db="${db#//}"
    db="${db%%\?*}"
    [[ "$db" != ":memory:" && -n "$db" ]] || { echo "backup: $DATABASE_URL is not a file; nothing to back up" >&2; exit 1; }
    [[ -f "$db" ]] || { echo "backup: SQLite database $db not found" >&2; exit 1; }
    command -v sqlite3 >/dev/null || { echo "backup: the sqlite3 CLI is required for a SQLite backup" >&2; exit 1; }
    [[ ! -e "$out/maidan.sqlite" ]] || { echo "backup: $out/maidan.sqlite already exists" >&2; exit 1; }
    echo "backup: snapshotting SQLite ($db) → $out/maidan.sqlite"
    # busy_timeout: wait out the server's write transaction instead of failing.
    target="$(printf '%s' "$out/maidan.sqlite" | sed "s/'/''/g")"
    sqlite3 -cmd ".timeout 30000" "$db" "VACUUM INTO '$target'"
    [[ "$(sqlite3 "$out/maidan.sqlite" "PRAGMA integrity_check")" == "ok" ]] \
      || { echo "backup: the snapshot failed its integrity check" >&2; exit 1; }
    ;;
  *)
    database=postgres
    echo "backup: dumping Postgres → $out/postgres.dump"
    pg_dump --format=custom --no-owner --no-privileges --file "$out/postgres.dump" "$DATABASE_URL"
    ;;
esac

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
database=$database
$([[ "$database" == sqlite ]] && echo sqlite_snapshot=maidan.sqlite || echo postgres_dump=postgres.dump)
artifact_backend=${ARTIFACT_BACKEND:-localfs}
artifact_archive=$([[ -f "$out/artifacts.tar.gz" ]] && echo artifacts.tar.gz || echo none)
MANIFEST

echo "backup: complete → $out"
