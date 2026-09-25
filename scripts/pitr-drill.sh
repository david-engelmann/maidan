#!/usr/bin/env bash
# Point-in-time recovery drill: prove, end to end, that a base backup plus the
# WAL archive restores Postgres to a chosen moment — rows written before it are
# back, rows written after it are not.
#
# `pg_dump` gives a recovery point only as fresh as the last dump. WAL
# archiving gives any point since the last base backup; this drill is the
# evidence that the archive and the restore procedure actually work, using the
# same settings as compose.pitr.yaml and the procedure in docs/Production.md.
#
# Needs only Docker. Usage: scripts/pitr-drill.sh [image]
#   image  defaults to pgvector/pgvector:pg16, the base of maidan-postgres.
set -euo pipefail

IMAGE="${1:-pgvector/pgvector:pg16}"
RUN_ID="pitr-drill-$$"
WORK="$(mktemp -d)"
ARCHIVE="${RUN_ID}-archive"
BACKUP="${RUN_ID}-backup"
PGDATA_RESTORE="${RUN_ID}-restore"

cleanup() {
  docker rm -f "${RUN_ID}-primary" "${RUN_ID}-restored" >/dev/null 2>&1 || true
  docker volume rm "$ARCHIVE" "$BACKUP" "$PGDATA_RESTORE" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

say() { printf '\n== %s\n' "$*"; }

psql_in() {
  local container="$1"
  shift
  docker exec "$container" psql -U maidan -d maidan -v ON_ERROR_STOP=1 -Atc "$@"
}

wait_ready() {
  local container="$1"
  for _ in $(seq 1 60); do
    if docker exec "$container" pg_isready -U maidan -d maidan >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  docker logs "$container" | tail -30
  echo "pitr-drill: $container never became ready" >&2
  return 1
}

docker volume create "$ARCHIVE" >/dev/null
docker volume create "$BACKUP" >/dev/null
docker volume create "$PGDATA_RESTORE" >/dev/null

say "primary with WAL archiving ($IMAGE)"
docker run -d --name "${RUN_ID}-primary" \
  -e POSTGRES_USER=maidan -e POSTGRES_PASSWORD=maidan -e POSTGRES_DB=maidan \
  -v "$ARCHIVE:/archive" \
  "$IMAGE" \
  -c wal_level=replica \
  -c archive_mode=on \
  -c "archive_command=test ! -f /archive/%f && cp %p /archive/%f" \
  -c archive_timeout=60 >/dev/null
docker exec "${RUN_ID}-primary" sh -c 'chown postgres:postgres /archive'
wait_ready "${RUN_ID}-primary"

psql_in "${RUN_ID}-primary" "CREATE TABLE drill (id int PRIMARY KEY, note text NOT NULL)"

say "base backup"
docker exec -u postgres "${RUN_ID}-primary" sh -c \
  'pg_basebackup -U maidan -D /tmp/base -X none -c fast && tar -C /tmp/base -cf - .' \
  > "$WORK/base.tar"

psql_in "${RUN_ID}-primary" "INSERT INTO drill VALUES (1, 'before the target')"
# The target is a moment strictly between the two writes, taken from the
# server's own clock so no host/container skew can move it.
sleep 1
TARGET="$(psql_in "${RUN_ID}-primary" "SELECT clock_timestamp()")"
sleep 1
psql_in "${RUN_ID}-primary" "INSERT INTO drill VALUES (2, 'after the target')"
# Close the segment holding both writes so the archive has it.
psql_in "${RUN_ID}-primary" "SELECT pg_switch_wal()" >/dev/null
for _ in $(seq 1 30); do
  failed="$(psql_in "${RUN_ID}-primary" "SELECT failed_count FROM pg_stat_archiver")"
  archived="$(psql_in "${RUN_ID}-primary" "SELECT archived_count FROM pg_stat_archiver")"
  [[ "$failed" == "0" ]] || { echo "pitr-drill: archive_command failing" >&2; exit 1; }
  [[ "$archived" -ge 2 ]] && break
  sleep 1
done
echo "target: $TARGET (archived segments: $archived)"
docker rm -f "${RUN_ID}-primary" >/dev/null

say "restore to the target from base backup + archive"
docker run --rm -i -v "$PGDATA_RESTORE:/restore" "$IMAGE" sh -c '
  set -e
  tar -C /restore -xf -
  touch /restore/recovery.signal
  chown -R postgres:postgres /restore
  chmod 700 /restore
' < "$WORK/base.tar"
docker run -d --name "${RUN_ID}-restored" \
  -e PGDATA=/restore \
  -v "$PGDATA_RESTORE:/restore" \
  -v "$ARCHIVE:/archive:ro" \
  "$IMAGE" \
  -c "restore_command=cp /archive/%f %p" \
  -c "recovery_target_time=$TARGET" \
  -c recovery_target_action=promote >/dev/null
wait_ready "${RUN_ID}-restored"
for _ in $(seq 1 30); do
  [[ "$(psql_in "${RUN_ID}-restored" "SELECT pg_is_in_recovery()")" == "f" ]] && break
  sleep 1
done

say "verify"
rows="$(psql_in "${RUN_ID}-restored" "SELECT string_agg(id::text, ',' ORDER BY id) FROM drill")"
if [[ "$rows" != "1" ]]; then
  echo "pitr-drill: FAILED — restored rows are [$rows], expected [1]" >&2
  docker logs "${RUN_ID}-restored" | tail -30 >&2
  exit 1
fi
echo "pitr-drill: OK — the write before $TARGET is back, the one after is not"
