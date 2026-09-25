# Cluster 414 retro — nothing grows without bound, nothing hangs forever

> Post-gate hardening · source record (no `v414.0.0` tag yet; the tag is the maintainer's) · PRs #1015, #1020 + close record

## Outcome

Every queue, cursor, connection and wait on the launch path now has a bound
or a timeout, and a failure that used to lose work is retried and then
repaired.

| Slice | PR | Result |
|-------|----|--------|
| 414.1 | #1015 | An abandoned delivery cursor no longer pins event-log retention; `idle_in_transaction_session_timeout` and `lock_timeout`; replica reads fenced on lag and poll staleness, with `MaidanReplicaLagHigh`; WebSocket message size and connection ceilings; per-tool MCP deadlines; a streamable-session reaper and ceiling; a lagging presence subscriber gets a fresh snapshot. |
| 414.2 | #1020 | A failed embedding batch is retried with backoff while the worker holds it, and a repair sweep embeds what retries could not save, under a Postgres advisory lock. |

## Decisions

- **Retention counts only cursors that have moved since the cutoff.** A
  consumer that stopped forever should not hold the log forever; one that
  resumes past the cutoff gets the snapshot path.
- **A replica is read only while it is fresh and close.** Stale after 2 s
  without a good poll, or more than 64 MiB behind; the primary answers
  otherwise.
- **Deadlines are per tool, not global.** 60 s by default; 330 s for the
  long-poll `wait_for_*` tools and bulk operations, which are slow on purpose.
- **Retry in place, repair later.** The worker's retries keep the queue
  bounded (it stops draining rather than growing); the sweep recovers work a
  restart or a persistent provider failure dropped, so a full reindex is only
  for a model change.

## What surprised us

- **One idle cursor could pin retention indefinitely.** The floor was the
  minimum over every cursor ever written, with nothing retiring one.
- **The embeddings doc pointed operators at a page that did not list the
  knobs.** It now carries the full table.
- **Local gates need `--no-fail-fast`.** A testcontainers flake at suite 361
  once hid thirty suites behind it.

## Carried forward

- None from this cluster; the verification-depth items stay in [[Open Work]].
