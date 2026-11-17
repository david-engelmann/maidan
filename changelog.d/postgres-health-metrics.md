### Postgres health on /metrics

- **Added:** six gauges, read from the Postgres catalogs on each scrape:
  - `maidan_postgres_xid_age`, the transactions since the oldest unfrozen id; Postgres refuses writes near 2 billion;
  - `maidan_postgres_dead_tuples` and `maidan_postgres_live_tuples`, whose ratio is the bloat vacuum has not reclaimed;
  - `maidan_postgres_wal_bytes`, the WAL write position, whose rate is the WAL written;
  - `maidan_postgres_replication_slot_retained_bytes` and `maidan_postgres_replication_slots_inactive`, for a slot nobody reads keeping WAL until the disk fills.

  A database that does not grant `pg_replication_slots` reports the other four, and SQLite reports none.
- **Added:** three alert rules on them, with promtool tests: `MaidanPostgresXidAgeHigh` (past a billion for 15 minutes), `MaidanPostgresDeadTuplesHigh` (a fifth of the rows dead for an hour) and `MaidanPostgresReplicationSlotRetainingWal` (more than 1 GiB held while a slot has no consumer, for 30 minutes).
