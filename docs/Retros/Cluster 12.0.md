# Cluster 12.0 retro — Outbox relay hardening

> Closing wave for Cluster 12.0 · target tag `v12.0.0`.

Cluster 12.0 bounded Postgres outbox relay retries and gave operators visibility into
poison/stuck rows without changing NOTIFY semantics.

## What shipped

- **PR #175** — Kickoff implementation spec ([[Clusters/Cluster 12.0]]).
- **PR #176** — Implementation bundle (12.0.1–12.0.4):
  - Migration `0014_outbox_quarantine.sql`; `quarantined_at`; relayable index.
  - `MAIDAN_OUTBOX_MAX_ATTEMPTS` (default 16); quarantine at cap.
  - Metrics `maidan_outbox_quarantined`, `maidan_outbox_oldest_pending_seconds`,
    `maidan_outbox_relay_total{result="quarantined"}`.
  - Decisions, Architecture, Production recovery runbook; tests.

## What was deferred

| To          | What                                              | Why                                      |
|-------------|---------------------------------------------------|------------------------------------------|
| Cluster 13  | Subscriber delivery ledger / cursors              | Separate delivery-semantics epic.        |
| Post-12.0   | HTTP admin replay for quarantined rows              | Manual SQL documented in Production.     |
| Post-12.0   | SQLite outbox parity                                | Postgres-only scope continues.           |

## Surprises

- None blocking; Cluster 11 tests provided a solid base for relay failure paths.

## Decisions

- **Quarantine, not delete** — rows stay in `maidan_outbox` for operator inspection.
- **Relayable = not published and not quarantined** — `maidan_outbox_pending` counts only relayable rows.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Outbox quarantine after max relay attempts              | `v12.0.0`          |
| `MAIDAN_OUTBOX_MAX_ATTEMPTS`                            | `v12.0.0`          |
| Outbox quarantine / oldest-pending metrics              | `v12.0.0`          |

## Risks identified + mitigated

- **Infinite relay retry on poison rows** — capped attempts + quarantine + metrics.

## Risks identified + still open

- **NOTIFY fire-and-forget** — unchanged.
- **Manual recovery only** — no replay API until a future cluster.

## Forward look

Next: **Cluster 13.0** — delivery contract and subscriber ledger. See [[Clusters/Cluster 13.0]].

## Acknowledgements

Solo cluster. Kickoff #175, implementation #176, this retro.
