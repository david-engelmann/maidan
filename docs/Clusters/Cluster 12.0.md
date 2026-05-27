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
| kickoff    | `docs: Cluster 12.0 kickoff plan`                                      | TBD   |
| 12.0.1     | `feat(maidan-store): outbox quarantine schema + cap attempts`          | TBD   |
| 12.0.2     | `feat(maidan-server): relay skips quarantined rows; oldest-pending gauge` | TBD   |
| 12.0.3     | `test: outbox quarantine and max-attempts integration`                 | TBD   |
| 12.0.4     | `docs: outbox failure modes in Production/Decisions`                   | TBD   |
| 12.0.retro | `docs(retro): Cluster 12.0 retrospective + v12.0.0 tag prep`            | TBD   |

## Order

1. **12.0.1** — migration (e.g. `quarantined_at TIMESTAMPTZ` on `maidan_outbox`);
   `MAIDAN_OUTBOX_MAX_ATTEMPTS` (default sensible, e.g. 16); when `attempts >= max`,
   set `quarantined_at` instead of retrying forever; `list_pending` excludes quarantined.
2. **12.0.2** — relay respects quarantine; metric
   `maidan_outbox_quarantined` (gauge count) and `maidan_outbox_oldest_pending_seconds`
   (age of min pending `created_at`); optional log at quarantine time.
3. **12.0.3** — testcontainers: poison publish → attempts increment → quarantine →
   pending count drops from relay batch; oldest-pending gauge moves; relay does not
   spin on quarantined row.
4. **12.0.4** — [[Production]] runbook (clear quarantine, replay row); [[Decisions]]
   ADR snippet; [[Architecture]] one-line relay state diagram.
5. **12.0.retro** + `v12.0.0` tag.

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
- Cluster 10.0 retro: [[Retros/Cluster 10.0]].
- Cluster 11.0 tests: `tests/outbox_http_e2e.rs`, `maidan-store/tests/outbox.rs`.
