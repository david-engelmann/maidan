# Cluster 12.0 — Outbox relay hardening

Cluster 11.0 closed coverage uplift at **`v11.0.0`**. Cluster 10.0 shipped the Postgres
transactional outbox and relay at **`v10.0.0`**, but failed publishes retry indefinitely:
`attempts` increments with no cap, and [[Production]] only documents manual triage for
high `maidan_outbox_pending`.

> **Goal:** Bound relay retries, surface poison rows and oldest-pending age for operators,
> and document failure modes — without changing NOTIFY semantics or claiming exactly-once.
>
> **Target tag:** `v12.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 12.0 kickoff plan` (#174, refinements in this PR)       | —     |
| 12.0.1     | `feat(maidan-store): outbox quarantine schema + cap attempts`          | TBD   |
| 12.0.2     | `feat(maidan-server): relay skips quarantined rows; oldest-pending gauge` | TBD   |
| 12.0.3     | `test: outbox quarantine and max-attempts integration`                 | TBD   |
| 12.0.4     | `docs: outbox failure modes in Production/Decisions`                   | TBD   |
| 12.0.retro | `docs(retro): Cluster 12.0 retrospective + v12.0.0 tag prep`            | TBD   |

## Order

### 12.0.1 — store + migration

- Migration **`0014_outbox_quarantine.sql`** (Postgres v14 in `migrate.rs`):
  - `ALTER TABLE maidan_outbox ADD COLUMN quarantined_at TIMESTAMPTZ;`
  - Partial index on pending non-quarantined rows (extend `idx_outbox_pending` predicate or add
    `idx_outbox_relayable` where `published_at IS NULL AND quarantined_at IS NULL`).
- **`maidan-store/src/postgres/outbox.rs`**:
  - `list_pending` / `count_pending`: exclude `quarantined_at IS NOT NULL`.
  - `quarantine(pool, outbox_id)` — set `quarantined_at = NOW()` (idempotent).
  - `count_quarantined(pool)` — gauge source.
  - `oldest_pending_created_at(pool)` — for age gauge (nullable if none pending).
  - After `record_attempt`, if `attempts >= max_attempts`, call `quarantine` (max from env,
    wired in 12.0.2 or passed into store helper).
- **`MAIDAN_OUTBOX_MAX_ATTEMPTS`** — default **16**; parse in `maidan-server` `main` (invalid
  → log + use default). Store on `AppState` for relay.

### 12.0.2 — relay + metrics

- **`OutboxRelay::relay_one` failure path**: after `record_attempt`, if at cap → `quarantine`
  + `counter!(maidan_outbox_relay_total, result=quarantined)` + warn log with `outbox_id`,
  `log_id`, `attempts`.
- **`metrics.rs`**: describe + set in `refresh_runtime_gauges`:
  - `maidan_outbox_quarantined` (gauge, count of quarantined unpublished rows or all
    quarantined — pick one and document).
  - `maidan_outbox_oldest_pending_seconds` (gauge, seconds since oldest relayable
    `created_at`, 0 when none).
- **`Production.md`**: env table row for `MAIDAN_OUTBOX_MAX_ATTEMPTS`; triage table rows for
  new metrics.

### 12.0.3 — tests

- **`maidan-store/tests/outbox.rs`**: attempts reach max → quarantine; `list_pending` skips
  quarantined; `count_quarantined`.
- **`maidan-server`**: extend `outbox_relay` unit test or `outbox_http_e2e` — `FailingBus`
  until quarantine, then pending relayable count 0, quarantined count 1.
- **`metrics` e2e**: scrape includes new gauges when pool + pending rows exist.

### 12.0.4 — docs

- [[Decisions]] — outbox quarantine ADR (bounded retry, manual recovery, no auto-delete).
- [[Architecture]] — relay states: pending → published | quarantined.
- [[Production]] — SQL to inspect quarantined rows; manual recovery (clear `quarantined_at`,
  reset `attempts`, or delete row + re-append — document safe path).

### 12.0.retro

- Retro, CHANGELOG `v12.0.0`, [[Capabilities]], tag.

## Exit criteria

- CI green on `main` (five required checks + coverage floor from 11.0).
- Relay stops retrying rows at configured max attempts; quarantined rows visible via
  metric and/or SQL.
- Ops docs describe triage and manual recovery.
- [[Retros/README]] includes Cluster 12.0; `v12.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Silent event loss if quarantine too aggressive | Default max high; metric + runbook; no auto-delete |
| Migration on live DB | Add nullable column; backfill none |
| Scope creep (DLQ replay API) | Quarantine + docs only; no admin HTTP in 12.0 |
| Duplicate NOTIFY unchanged | Document at-least-once unchanged |

## Out of scope

- Consumer dedup table / delivery ledger (Cluster 13.0).
- SQLite outbox parity.
- NOTIFY / LISTEN guaranteed delivery.
- Coverage floor bump.
- Automatic quarantine replay HTTP API.

## Follow-on clusters (not this wave)

| Cluster | Tag | Theme |
|---------|-----|--------|
| **13.0** | `v13.0.0` | Delivery contract & subscriber ledger |
| **14.0** | `v14.0.0` | Epic pick (SQLite semantic, SQLite outbox, MCP subscribe, S3 multipart) |

## References

- Outbox relay: `maidan-server/src/outbox_relay.rs`, `maidan-store/src/postgres/outbox.rs`.
- Migration v13: `migrations/postgres/0013_outbox.sql`.
- Cluster 10.0 retro: [[Retros/Cluster 10.0]].
- Cluster 11.0 tests: `tests/outbox_http_e2e.rs`, `maidan-store/tests/outbox.rs`.
