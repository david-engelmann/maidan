# Cluster 11.0 retro — Coverage 11%

> Closing wave for Cluster 11.0 · target tag `v11.0.0`.

Cluster 11.0 raised the CI line-coverage floor to **11.0%** with behavioral tests
focused on Postgres outbox and relay paths shipped in Cluster 10.0.

## What shipped

- **PR #172** — Cluster kickoff plan ([[Clusters/Cluster 11.0]]).
- **PR #173** — Implementation bundle (11.0.1–11.0.3):
  - `maidan-store` outbox integration tests (`record_attempt`, `mark_published`, ordering).
  - `maidan-bus::test_support` (`FailingBus`, `RecordingBus`).
  - `maidan-server` `publish` deferral unit tests; relay failure path; HTTP outbox e2e;
    `/metrics` outbox gauges; `/ui/` static e2e; subscribe metrics label sweep.
  - `COVERAGE_MIN_LINES` **10.5 → 11.0**; [[Operations]] bump notes.

## What was deferred

| To          | What                                              | Why                                      |
|-------------|---------------------------------------------------|------------------------------------------|
| Cluster 12  | Outbox max-attempts / quarantine / oldest-pending age | Planned sequel to v10.0 relay ops gap.   |
| Cluster 13  | Subscriber delivery ledger                          | Larger delivery-semantics epic.          |
| Post-11.0   | NOTIFY guaranteed delivery                        | Out of scope for coverage cluster.       |
| Post-11.0   | Playwright / frontend build                         | No JS bundle; static `/ui/` only.        |

## Surprises

- Prometheus omits zero-valued counters until first increment — outbox relay metric
  e2e must run a successful relay tick before asserting `maidan_outbox_relay_total`.
- `cargo fmt` import order for `maidan_types::EventFilter` blocked first CI green run.

## Decisions

- **CI is source of truth** for floor bumps; local `llvm-cov` used for sanity only.
- **`test_support` in `maidan-bus`** — shared bus doubles for server/store tests.
- **No production behavior changes** — tests and floor only.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| CI line-coverage floor at 11.0%                           | `v11.0.0`          |
| Outbox/relay/publish deferral test coverage               | `v11.0.0`          |
| `GET /ui/` static HTML e2e                                | `v11.0.0`          |

## Risks identified + mitigated

- **Outbox failure paths untested** — relay failure, publish deferral, and store outbox
  helpers now covered by unit and integration tests.

## Risks identified + still open

- **Stuck pending rows without cap** — relay retries forever; Cluster 12 adds max attempts.
- **NOTIFY fire-and-forget** — unchanged.
- **Coverage still modest** — 11% floor; further uplift is incremental.

## Forward look

Next: **Cluster 12.0** — outbox relay hardening (max attempts, oldest-pending age,
quarantine ops). See [[Clusters/Cluster 12.0]].

## Acknowledgements

Solo cluster. Kickoff #172, implementation #173, this retro.
