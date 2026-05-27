# Cluster 11.0 — Coverage 11%

Cluster 10.0 closed the Postgres transactional outbox at **`v10.0.0`**. Cluster 9.0
raised `COVERAGE_MIN_LINES` to **10.5** at **`v9.0.0`**; retros since 5.0 repeatedly
defer a measured bump toward **11%+** (`COVERAGE_MIN_LINES=11.0` failed on first
attempt in Cluster 5). Delivery clusters 6–10 added substantial outbox and relay code
with integration tests but modest unit coverage on failure paths.

> **Goal:** Add focused unit and integration tests in high-risk, recently touched
> crates (especially outbox and relay); re-measure line coverage on green `main`;
> raise `COVERAGE_MIN_LINES` to slightly below the new measurement; document the
> bump in [[Operations]].
>
> **Target tag:** `v11.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| 11.0.1–3   | `test: Cluster 11.0 coverage depth and 11% CI floor` (#173)            | —     |
| kickoff    | `docs: Cluster 11.0 kickoff plan` (#172)                                 | —     |
| 11.0.retro | `docs(retro): Cluster 11.0 retrospective + v11.0.0 tag prep` (this PR) | —     |

## Order

1. **11.0.1** — add tests (no blanket `unwrap` padding):
   - **`maidan-store` (Postgres):** `record_attempt` increments `attempts`; row stays
     pending; `mark_published` clears count; `list_pending` limit/ordering; multiple
     appends enqueue multiple rows.
   - **`maidan-server`:** `outbox_relay` failure path (failed publish → pending +
     `maidan_outbox_relay_total{result="failed"}`); `publish` skips direct
     `bus.publish` when `outbox_relay` is true; `/metrics` scrape includes
     `maidan_outbox_pending` / `maidan_outbox_relay_total` when `outbox_pool` is set.
   - **Stretch** (only if measurement still short of 11%): `maidan-fsm` guards,
     `subscribe_metrics` edge labels — behavior assertions only.
2. **11.0.2** — on a green branch, use CI `coverage (llvm-cov)` (or reproduce locally);
   set `COVERAGE_MIN_LINES` in `.github/workflows/ci.yml` **below** measured
   (bump-below-measured policy from Cluster 5/9; target **11.0** only if CI green).
3. **11.0.3** — [[Operations]]: record CI run id, new floor, re-measure instructions.
4. **11.0.retro** + `v11.0.0` tag.

## Exit criteria

- CI green on `main` (five required checks + raised coverage floor).
- `COVERAGE_MIN_LINES` reflects a fresh measurement documented in [[Operations]].
- No change to outbox relay behavior or NOTIFY semantics.
- [[Retros/README]] includes Cluster 11.0; `v11.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| `11.0` gate fails again | Measure first; set floor below green `main` (e.g. 10.75) |
| Flaky coverage on different hosts | CI is source of truth; document run id |
| Low-value tests | Focus on behavior assertions, not line-padding |
| Local `llvm-cov` too slow | Rely on CI measurement for floor bump |

## Out of scope

- Outbox max-attempts / quarantine / DLQ (Cluster 12.0).
- Consumer dedup table / delivery ledger (Cluster 13.0).
- SQLite outbox, semantic search, MCP `resources/subscribe` streaming.
- Codecov workflow changes (shipped in 5.0).
- NOTIFY guarantees or exactly-once end-to-end.

## Follow-on clusters (not this wave)

| Cluster | Tag | Theme |
|---------|-----|--------|
| **12.0** | `v12.0.0` | Outbox relay hardening (max attempts, oldest-pending age, poison rows) |
| **13.0** | `v13.0.0` | Delivery contract & subscriber ledger |
| **14.0** | `v14.0.0` | Epic pick at 13 retro (SQLite semantic, SQLite outbox, MCP subscribe, S3 multipart) |

## References

- Cluster 9.0 coverage playbook: [[Clusters/Cluster 9.0]], [[Retros/Cluster 9.0]].
- Cluster 10.0 outbox code: `maidan-store/src/postgres/outbox.rs`,
  `maidan-server/src/outbox_relay.rs`, `tests/outbox_relay_e2e.rs`.
- CI coverage job: `.github/workflows/ci.yml` (`COVERAGE_MIN_LINES`).
