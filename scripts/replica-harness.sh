#!/usr/bin/env bash
# Local Postgres primary + streaming-replica harness (Cluster 261, Program D).
#
# Stands up a real primary and a hot-standby (streaming replication) on the
# pgvector/pgvector:pg17 image — the validation vehicle for LSN causality-token
# read-replica routing. Infra lives here; the assertions live in the #[ignore]d
# Rust tests, which connect via the two URLs this prints (the loadgen pattern).
#
# Usage:
#   scripts/replica-harness.sh up      # bring up the pair, print the two URLs
#   eval "$(scripts/replica-harness.sh up)"   # ...and export them into your shell
#   scripts/replica-harness.sh down    # tear it all down
#
# Prints (on `up`):
#   export MAIDAN_PRIMARY_URL=postgres://postgres@localhost:<p>/postgres
#   export MAIDAN_REPLICA_URL=postgres://postgres@localhost:<s>/postgres
set -uo pipefail

NET=maidan-replica-net
P=maidan-replica-primary
S=maidan-replica-standby
IMG=pgvector/pgvector:pg17
PPORT="${MAIDAN_PRIMARY_PORT:-54321}"
SPORT="${MAIDAN_REPLICA_PORT:-54322}"

down() { docker rm -f "$P" "$S" >/dev/null 2>&1; docker network rm "$NET" >/dev/null 2>&1; }

up() {
  down
  docker network create "$NET" >/dev/null
  docker run -d --name "$P" --network "$NET" -p "${PPORT}:5432" \
    -e POSTGRES_PASSWORD=pw -e POSTGRES_HOST_AUTH_METHOD=trust "$IMG" \
    -c wal_level=replica -c max_wal_senders=10 -c wal_keep_size=64 -c hot_standby=on >/dev/null
  until docker exec "$P" pg_isready -U postgres >/dev/null 2>&1; do sleep 1; done
  # Replication needs its own pg_hba entry — host-auth trust does NOT cover it.
  docker exec "$P" bash -c 'echo "host replication all all trust" >> /var/lib/postgresql/data/pg_hba.conf'
  docker exec "$P" psql -U postgres -c "SELECT pg_reload_conf();" >/dev/null

  # Standby: pg_basebackup (-R writes standby.signal + primary_conninfo), then run
  # postgres as the postgres user (it refuses to run as root).
  docker run -d --name "$S" --network "$NET" -p "${SPORT}:5432" --user postgres --entrypoint bash "$IMG" -c "
    set -e; export PGDATA=/tmp/standby-data
    rm -rf \"\$PGDATA\"; mkdir -p \"\$PGDATA\"; chmod 700 \"\$PGDATA\"
    until pg_basebackup -h $P -U postgres -D \"\$PGDATA\" -Fp -Xs -R -P; do sleep 1; done
    exec postgres -D \"\$PGDATA\" -c hot_standby=on
  " >/dev/null
  for _ in $(seq 1 60); do docker exec "$S" pg_isready -U postgres >/dev/null 2>&1 && break; sleep 1; done

  echo "export MAIDAN_PRIMARY_URL=postgres://postgres@localhost:${PPORT}/postgres"
  echo "export MAIDAN_REPLICA_URL=postgres://postgres@localhost:${SPORT}/postgres"
}

case "${1:-up}" in
  up) up ;;
  down) down; echo "replica harness down" ;;
  *) echo "usage: $0 [up|down]" >&2; exit 2 ;;
esac
