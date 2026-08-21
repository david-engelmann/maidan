#!/usr/bin/env bash
# Maidan restore (Cluster 260, Program D — disaster recovery). Inverse of backup.sh.
#
# Restores a backup directory into a target Postgres (+ localfs artifact root).
# GUARDRAIL: refuses to restore into a NON-EMPTY database unless --force is given,
# so you can't silently clobber a live deployment. pg_restore runs with
# --clean --if-exists when --force is set.
#
# Usage:
#   DATABASE_URL=postgres://…  scripts/restore.sh backups/<timestamp> [--force]
#   ARTIFACT_LOCALFS_ROOT=/var/lib/maidan/artifacts  DATABASE_URL=…  \
#     scripts/restore.sh backups/<timestamp> --force
set -euo pipefail

: "${DATABASE_URL:?set DATABASE_URL to the TARGET Postgres connection string}"

src="${1:?usage: restore.sh <backup-dir> [--force]}"
force="${2:-}"
[[ -f "$src/postgres.dump" ]] || { echo "restore: $src/postgres.dump not found" >&2; exit 1; }

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

if [[ -f "$src/artifacts.tar.gz" ]]; then
  root="${ARTIFACT_LOCALFS_ROOT:?artifacts.tar.gz present — set ARTIFACT_LOCALFS_ROOT to restore into}"
  mkdir -p "$root"
  echo "restore: unpacking artifacts → $root"
  tar -xzf "$src/artifacts.tar.gz" -C "$root"
fi

echo "restore: complete. Verify /health/ready before serving traffic."
