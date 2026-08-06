# Cluster 166.0 retro — SQLite pragmas + webhook fan-out

> Tag **`v166.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> First cluster of Arc 2 (perf + CI/CD).

## What shipped

- **R3 (correctness):** `foreign_keys` / `busy_timeout` / `journal_mode = WAL`
  now run in the SQLite pool's `after_connect` (`sqlite_pool_options_with`), so
  every pooled connection gets them — not just the first. `main.rs` uses the new
  builder and drops the one-shot `configure_pool` call.
- **H1 (perf):** the webhook worker queries only the event's workspace's enabled
  subscriptions (indexed) instead of scanning every workspace on every event, and
  builds the payload lazily on first match.

## What was deferred / not covered

| Item | Why |
|------|-----|
| Rest of Arc 2 perf (H6, H4, R2, H2, R1) | Next cluster. |
| CI/CD workflow speedups | Can't run/validate during the GitHub Actions outage — deferred until it recovers. |

## Surprises

- **The pragma bug was invisible in tests.** Every test pool is tiny and
  effectively single-connection, and several set `PRAGMA foreign_keys = ON`
  explicitly on the pool — so FKs *were* enforced there. The bug only bit the
  production multi-connection pool. The new test had to use a **file-backed** DB
  and **hold** several connections at once to force the pool to expose more than
  one — `:memory:` gives each connection its own database, which would have
  hidden it again.

## Decisions

- **`after_connect`, not a post-connect sweep.** Per-connection settings belong
  in the per-connection hook; a one-shot sweep can only ever touch whichever
  connection the pool hands back.
- **Lazy payload in the webhook worker** — the old code serialized the payload on
  every event even when no subscription matched; now it's built on first match
  (and reused for the mention-webhook path).

## Capability table extension

| Fix | Where |
|-----|-------|
| Per-connection SQLite pragmas; per-workspace webhook fan-out | `sqlite_vec.rs`, `webhook_worker.rs` |

## Risks identified + still open

- **Low.** R3 strengthens integrity (FKs now enforced everywhere); H1 is a
  narrower query with identical delivery semantics (webhook e2e unchanged).
  Shipped during the GitHub Actions outage; re-run CI on `main` when recovered.

## Forward look

Arc 2 continues with the remaining perf items — H6 (cache the embedding
model→table lookup), H4 (JOIN the payload into the outbox `list_pending` + batch
`mark_published`), R2 (evict the in-memory rate-limiter map), H2 (coalesce
delivery-cursor writes), R1 (env-tunable `BROADCAST_CAP`) — then the CI/CD
workflow speedups once GitHub Actions is back.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
