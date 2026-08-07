# Cluster 168.0 retro — outbox relay round-trips + tunable broadcast cap

> Tag **`v168.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 2 (perf), part 3.

## What shipped

- **H4:** `outbox::list_pending` now JOINs `maidan_events.payload`, so the relay
  publishes straight from the pending row instead of a per-row
  `get_stored_event`. Successfully-published rows are marked in one
  `mark_published_batch` after the loop rather than a round-trip each. Both
  backends (Postgres `id = ANY($1)`, SQLite dynamic `IN (…)`).
- **R1:** `MAIDAN_BUS_BROADCAST_CAP` (default 1024) via a shared
  `maidan_bus::broadcast_cap_from_env()`, wired into the event bus + presence +
  resource broadcast channels.
- **Hotfix:** two `unwrap()`s in `webhook_worker.rs` (Cluster 166) rewritten with
  `let-else` — they were failing the strict lint and holding `main` red.

## What was deferred / not covered

| Item | Why |
|------|-----|
| H2 (delivery-cursor coalesce) | Different code path (per-subscriber cursor writes); own cluster. |
| CI/CD workflow speedups | Now unblocked; next cluster. |

## Surprises

- **`main` was red the moment CI came back.** Clusters 159–167 merged during the
  GitHub Actions outage with local validation only. My local clippy was
  `--all-targets --workspace -D warnings` — which does **not** enable the
  `restriction` lints. The `lint` CI job runs a **separate** step,
  `cargo clippy --workspace --lib --bins -- -D clippy::unwrap_used -D
  clippy::expect_used`, and it caught a Cluster 166 `unwrap()`. Lesson recorded
  in memory (`maidan-strict-unwrap-lint`): validate with **both** clippy
  invocations before any merge, especially a CI-less batch.
- **The payload was already one JOIN away.** `maidan_outbox.log_id` is a FK to
  `maidan_events.id`, so an INNER JOIN never drops a legitimate pending row — the
  transactional outbox writes the event and the outbox row in the same tx.

## Decisions

- **Batch-mark after the loop, not per row.** A crash between publish and the
  batch mark re-publishes the whole batch next tick — acceptable under the
  at-least-once contract (consumers dedup on `log_id`). Kept the per-row
  `mark_published` for the store tests / other callers.
- **Shared env helper, not per-module consts.** One `broadcast_cap_from_env()`
  in `lib.rs` mirrors the existing `max_attempts_from_env()` pattern; the three
  `const BROADCAST_CAP` definitions collapsed into it.
- **Fold the hotfix into 168.** The unwrap fix is a one-file, delivery-adjacent
  change and it unblocks `main`; a standalone hotfix PR would have cost a full
  CI cycle for two lines. Called out here and in the plan.

## Capability table extension

| Fix | Where |
|-----|-------|
| Outbox relay: JOIN payload + batch mark_published; env-tunable broadcast cap; webhook unwrap hotfix | `store/*/outbox.rs`, `server/outbox_relay.rs`, `bus/lib.rs`, `server/webhook_worker.rs` |

## Risks identified + still open

- **Low.** H4 is transparent (same rows relayed, fewer round-trips); the only
  behavioral delta is the wider duplicate window on crash, already tolerated. R1
  defaults to the prior 1024. The hotfix is a pure lint/correctness fix.

## Forward look

Arc 2 finishes with **H2** (coalesce per-subscriber delivery-cursor writes) and
the **CI/CD workflow speedups** (native arm64 runner for the QEMU-slow release
image build, `gha` cargo cache, `trivy` scan) — both unblocked now that GitHub
Actions is back.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
