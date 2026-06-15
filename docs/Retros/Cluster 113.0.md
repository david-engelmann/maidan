# Cluster 113.0 retro — Backend parity harness

> Tag **`v113.0.0`**. Third cluster of Phase XXI (correctness & coverage).

## What shipped

- **Static lockstep guard** (`maidan-store/tests/backend_parity.rs`,
  Docker-free, runs in the required `unit tests` job). (113.0.1, #312)
  - **`migrations_stay_in_lockstep`** — every migration *slug* (filename minus
    the `NNNN_` prefix and any `_up`/`_down`) exists for both backends, modulo
    an allowlist.
  - **`store_modules_stay_in_lockstep`** — every `src/{postgres,sqlite}/*.rs`
    module exists for both backends, modulo an allowlist.
  - **Allowlist (rationale in-code):** Postgres-only `outbox_quarantine` (a
    separate migration on PG, folded into `0013_outbox` on SQLite — both
    backends *have* quarantine) and SQLite-only `pragmas` (per-connection
    `PRAGMA` setup; no PG equivalent). A new unmatched migration/module fails
    CI until the author adds the counterpart or consciously extends the
    allowlist with a reason.
  - Plus a unit test pinning the slug parser.
- **Broadened cross-dialect snapshot** — `run_parity_scenario` /
  `ParitySnapshot` (driven by the existing `dialect_parity.rs` identity test)
  now also exercises an FSM transition (`Open → InReview` via
  `transition_thread`), a message edit (+ recorded edit count), and a
  reaction, so the "both backends return identical results" assertion covers
  that wider surface. Verified against a real Postgres testcontainer.

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| Cluster 114  | Coverage-floor ratchet + fuzz | `COVERAGE_MIN_LINES` 11→25→40 and envelope (JSON-RPC / MCP) fuzzing are the next cluster. |
| (ongoing)    | Column-level schema parity | The guard is file/slug-level; per-column type parity between the two SQL dialects is checked behaviorally by `dialect_parity` rather than by static schema diffing (the dialects intentionally differ — `JSONB` vs `TEXT`, `TIMESTAMPTZ` vs `TEXT`). |

## Surprises

- **The two backends' migration *numbering* diverged long ago** — e.g. SQLite
  `0028_oauth_codes` vs Postgres `0029_oauth_codes`, SQLite
  `0025_automation_deliveries` vs Postgres `0026_…`. An index-based lockstep
  check would be useless; comparing *slugs* (feature names) is the only
  invariant that holds. This is exactly the kind of drift the guard now pins.
- **The backends are already remarkably aligned** — only two legitimate
  divergences across 29 migration slugs and ~38 store modules. The guard
  mostly protects *future* work from silent single-backend additions.

## Decisions

- **Slug-based parity, not index- or content-based.** Filenames carry the
  feature identity; numeric prefixes are backend-local ordering and the SQL
  bodies legitimately differ per dialect. Comparing slugs with a small
  rationale-documented allowlist is the right granularity. No
  [[Architecture]] change.
- **Static guard lives in `unit tests`, not a new CI job.** Running it as an
  ordinary `#[test]` keeps it in an already-required check — no
  branch-protection change, no Docker dependency.

## Capability table extension

| Capability | Where |
|------------|-------|
| Migration + store-module lockstep guard (allowlisted) | `maidan-store/tests/backend_parity.rs` |
| Cross-dialect identity over FSM / edit / reaction surface | `maidan-store/tests/{common/mod.rs,dialect_parity.rs}` |

## Risks identified + mitigated

- **Silent single-backend drift.** A migration or store module added to one
  backend and forgotten on the other previously passed CI; it now fails the
  lockstep guard.

## Risks identified + still open

- **Allowlist rot.** A future genuinely-shared feature could be wrongly
  allowlisted to silence the guard. Mitigated by requiring an in-code
  rationale comment for every allowlist entry (reviewed in PR).

## Forward look

Phase **XXI** continues with **Cluster 114 — coverage uplift + fuzz**: raise
`COVERAGE_MIN_LINES` in steps (11 → 25 → 40) and add fuzz / round-trip tests
on the JSON-RPC / MCP envelope surface.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
