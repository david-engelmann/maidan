#!/usr/bin/env bash
# Maidan restore. Inverse of backup.sh.
#
# Restores a backup directory into a target Postgres or SQLite database (+ a
# localfs artifact root).
# GUARDRAIL: refuses to restore into a NON-EMPTY database unless --force is given,
# so you can't silently clobber a live deployment. pg_restore runs with
# --clean --if-exists when --force is set.
#
# SQLite: stop the server first. The snapshot replaces the database file, and
# the old -wal and -shm files are removed with it: left behind, SQLite would
# replay the old database's uncheckpointed pages onto the restored one.
#
# Usage:
#   DATABASE_URL=postgres://…  scripts/restore.sh backups/<timestamp> [--force]
#   DATABASE_URL=sqlite:///data/maidan.db  scripts/restore.sh backups/<timestamp> [--force]
#   ARTIFACT_LOCALFS_ROOT=/var/lib/maidan/artifacts  DATABASE_URL=…  \
#     scripts/restore.sh backups/<timestamp> --force
set -euo pipefail

: "${DATABASE_URL:?set DATABASE_URL to the TARGET database URL (postgres:// or sqlite:)}"

src="${1:?usage: restore.sh <backup-dir> [--force]}"
force="${2:-}"

case "$DATABASE_URL" in
  sqlite:*)
    [[ -f "$src/maidan.sqlite" ]] || { echo "restore: $src/maidan.sqlite not found (is this a Postgres backup?)" >&2; exit 1; }
    db="${DATABASE_URL#sqlite:}"
    db="${db#//}"
    db="${db%%\?*}"
    [[ "$db" != ":memory:" && -n "$db" ]] || { echo "restore: $DATABASE_URL is not a file" >&2; exit 1; }
    command -v sqlite3 >/dev/null || { echo "restore: the sqlite3 CLI is required for a SQLite restore" >&2; exit 1; }
    [[ "$(sqlite3 "$src/maidan.sqlite" "PRAGMA integrity_check")" == "ok" ]] \
      || { echo "restore: $src/maidan.sqlite failed its integrity check" >&2; exit 1; }
    tables=0
    if [[ -f "$db" ]]; then
      tables="$(sqlite3 "$db" "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'")"
    fi
    if [[ "${tables:-0}" -gt 0 && "$force" != "--force" ]]; then
      echo "restore: target database is not empty ($tables tables). Stop the server, then re-run with --force to overwrite." >&2
      exit 1
    fi
    echo "restore: replacing $db with $src/maidan.sqlite"
    mkdir -p "$(dirname "$db")"
    cp "$src/maidan.sqlite" "$db.restoring"
    rm -f "$db-wal" "$db-shm"
    mv -f "$db.restoring" "$db"
    ;;
  *)
    [[ -f "$src/postgres.dump" ]] || { echo "restore: $src/postgres.dump not found (is this a SQLite backup?)" >&2; exit 1; }

    # Is the target empty? (no user tables in the public schema)
    tables="$(psql "$DATABASE_URL" -tAc \
      "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")"
    if [[ "${tables:-0}" -gt 0 && "$force" != "--force" ]]; then
      echo "restore: target database is not empty ($tables tables). Re-run with --force to overwrite." >&2
      exit 1
    fi

    echo "restore: loading Postgres from $src/postgres.dump"
    if [[ "$force" == "--force" ]]; then
      pg_restore --clean --if-exists --no-owner --no-privileges --dbname "$DATABASE_URL" "$src/postgres.dump"
    else
      pg_restore --no-owner --no-privileges --dbname "$DATABASE_URL" "$src/postgres.dump"
    fi
    ;;
esac

if [[ -f "$src/artifacts.tar.gz" ]]; then
  root="${ARTIFACT_LOCALFS_ROOT:?artifacts.tar.gz present — set ARTIFACT_LOCALFS_ROOT to restore into}"
  mkdir -p "$root"
  echo "restore: unpacking artifacts → $root"
  tar -xzf "$src/artifacts.tar.gz" -C "$root"
fi

echo "restore: complete. Verify /health/ready before serving traffic."
