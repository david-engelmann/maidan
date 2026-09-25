# Cluster 417 retro — disaster recovery that is actually tested

> Post-gate hardening · source record (no `v417.0.0` tag yet; the tag is the maintainer's) · PR #1026 + close record

## Outcome

Postgres can be restored to any moment since its last base backup, and CI
proves it on every PR. Before, the recovery point was only as fresh as the
last `pg_dump`, and the docs named WAL archiving without settings, procedure
or evidence.

| Slice | PR | Result |
|-------|----|--------|
| 417.1 | #1026 | `compose.pitr.yaml` (WAL archiving), the restore procedure in Production.md, and `scripts/pitr-drill.sh`, which CI runs against the image built from the tree. |

## Decisions

- **The drill proves a boundary, not just a restore.** It writes a row, notes
  the time, writes another, restores to that time, and fails unless exactly
  the first row is back. A restore that replayed everything would pass a
  weaker check; this one fails it.
- **The target time comes from Postgres's own clock**, so host/container skew
  cannot move it.
- **The image owns the archive mount point.** A fresh named volume at
  `/archive` is otherwise root-owned, and `archive_command` fails silently.
- **Production archives go off the host.** The docs point `archive_command` at
  WAL-G, pgBackRest or a managed service; an archive on the database's own
  disk survives a bad deploy, not a lost disk.

## What surprised us

- **Archiving failed silently on the first overlay run**, because of the
  volume ownership above. `pg_stat_archiver.failed_count` is the check, and the
  drill now fails on it.

## Carried forward

- Promote `pitr drill` to a required check after a clean stretch.
