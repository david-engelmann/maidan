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
# replay the old database's uncheckpointed pages onto the restored one. With
# --force the target is not opened at all, so a corrupt file can be replaced.
# The restored file keeps the owner and mode of the one it replaces; a new
# target belongs to whoever runs this, so run it as the server's user (or
# chown the file after).
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

# SQLx percent-decodes the path in a sqlite: URL (so a file name can hold `?`
# or `#`), and so must this, or the restore lands beside the file the server
# opens. An escape that is not two hex digits stays as written, as in SQLx.
# backup.sh has the same function.
percent_decode() {
  local rest="$1" out="" byte
  while [[ "$rest" == *%* ]]; do
    out+="${rest%%\%*}"
    rest="${rest#*%}"
    if [[ "${rest:0:2}" =~ ^[0-9A-Fa-f]{2}$ ]]; then
      printf -v byte '%b' "\\x${rest:0:2}"
      out+="$byte"
      rest="${rest:2}"
    else
      out+="%"
    fi
  done
  printf '%s' "$out$rest"
}

case "$DATABASE_URL" in
  sqlite:*)
    [[ -f "$src/maidan.sqlite" ]] || { echo "restore: $src/maidan.sqlite not found (is this a Postgres backup?)" >&2; exit 1; }
    db="${DATABASE_URL#sqlite:}"
    db="${db#//}"
    db="${db%%\?*}"
    [[ "$db" != ":memory:" && -n "$db" ]] || { echo "restore: $DATABASE_URL is not a file" >&2; exit 1; }
    db="$(percent_decode "$db")"
    command -v sqlite3 >/dev/null || { echo "restore: the sqlite3 CLI is required for a SQLite restore" >&2; exit 1; }
    [[ "$(sqlite3 "$src/maidan.sqlite" "PRAGMA integrity_check")" == "ok" ]] \
      || { echo "restore: $src/maidan.sqlite failed its integrity check" >&2; exit 1; }
    if [[ "$force" != "--force" && -f "$db" ]]; then
      tables="$(sqlite3 "$db" "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'" 2>/dev/null)" \
        || { echo "restore: $db is not a readable SQLite database. Stop the server, then re-run with --force to replace it." >&2; exit 1; }
      if [[ "$tables" -gt 0 ]]; then
        echo "restore: target database is not empty ($tables tables). Stop the server, then re-run with --force to overwrite." >&2
        exit 1
      fi
    fi
    # A copy made as root would leave a root-owned file the server cannot
    # write. GNU stat takes -c, BSD stat -f.
    owner_mode=""
    if [[ -e "$db" ]]; then
      owner_mode="$(stat -c '%u:%g %a' "$db" 2>/dev/null || stat -f '%u:%g %Lp' "$db")"
    fi
    echo "restore: replacing $db with $src/maidan.sqlite"
    mkdir -p "$(dirname "$db")"
    cp "$src/maidan.sqlite" "$db.restoring"
    if [[ -n "$owner_mode" ]] \
      && ! { chown "${owner_mode% *}" "$db.restoring" && chmod "${owner_mode#* }" "$db.restoring"; }; then
      rm -f "$db.restoring"
      echo "restore: cannot give the restored file the owner and mode of $db ($owner_mode); run as root or as its owner" >&2
      exit 1
    fi
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
