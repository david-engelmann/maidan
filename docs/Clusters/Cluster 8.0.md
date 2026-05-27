# Cluster 8.0 — Bus hydrate observability

Cluster 7.0 closed bus pointer delivery at **`v7.0.0`**. Postgres `NOTIFY` now
carries `log_id_v1` pointers and the listener hydrates from `maidan_events`, but
hydrate failures are only visible in logs (`tracing::warn` on drop) — not on
`/metrics`. Cluster 6.0 added subscribe lag, replay, indexer age, and listener
health gauges; the pointer path still lacks operator-facing counters for
`HydrateNotFound`, `HydrateFailed`, and malformed notify payloads.

> **Goal:** Expose hydrate outcomes on Prometheus (`maidan_bus_notify_hydrate_total`
> with fixed `result` labels), document alerting and troubleshooting, and prove
> counters move in tests — without changing at-most-once semantics or adding an
> outbox.
>
> **Target tag:** `v8.0.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| 8.0.1–3   | `feat: Cluster 8.0 bus hydrate observability` (#165)                   | —     |
| kickoff   | `docs: Cluster 8.0 kickoff plan` (#164)                                 | —     |
| 8.0.retro | `docs(retro): Cluster 8.0 retrospective + v8.0.0 tag prep` (this PR)   | —     |

## Order

1. **8.0.1** — add `HydrateStats` (or extend `ListenerHealth`) in `maidan-bus`:
   increment `maidan_bus_notify_hydrate_total{result}` for `ok`, `not_found`,
   `failed`, `invalid_payload` in `decode_notify_payload` / `hydrate_envelope`.
   Wire through `PostgresBus::hydrate_stats()` (mirror `listener_health()`).
   Export on server `/metrics` in `maidan-server/src/metrics.rs` (same pattern as
   `maidan_bus_listener_ok`). Register descriptions in `metrics::init()`.
2. **8.0.2** — [[Production]] alert table (hydrate spike → DB connectivity,
   publish-order audit, cross-check `maidan_subscribe_replay_total`); [[Operations]]
   troubleshooting subsection; [[Architecture]] pointer-flow diagram references
   hydrate series. Update OpenAPI `/metrics` note if metric names are documented
   there.
3. **8.0.3** — tests in `maidan-bus/tests/postgres_bus.rs` (testcontainers):
   assert `not_found` (or equivalent) increments when NOTIFY references a missing
   `log_id`; keep existing pointer round-trip and large-event tests green.
   Optional: scrape `/metrics` in a server e2e if cheap.
4. **8.0.retro** + `v8.0.0` tag.

## Exit criteria

- CI green on `main` (five required checks + coverage floor from 5.0).
- `/metrics` exposes `maidan_bus_notify_hydrate_total{result}` with fixed cardinality.
- At least one test proves a non-`ok` hydrate counter increments.
- [[Production]] and [[Operations]] describe hydrate failure symptoms and actions.
- [[Retros/README]] includes Cluster 8.0; `v8.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Low hydrate failure rate in prod | Test-driven proof; docs tie spikes to DB / ordering bugs |
| Metrics only on Postgres bus | Document that `InMemoryBus` has no hydrate path |
| Label cardinality explosion | Fixed `result` enum only; no `log_id` labels |
| Scope creep into outbox | Explicit out-of-scope below |

## Out of scope

- Outbox pattern, transactional NOTIFY, or guaranteed at-least-once semantics.
- Changing pointer vs legacy publish semantics (`v7.0.0` behavior frozen).
- Coverage floor bump to 11%+ (separate cluster; see alternative below).
- Per-model embedding tables / SQLite semantic search.
- SSE for MCP `resources/subscribe` (Cluster B deferral).

## Dependencies

- **8.0.1** before **8.0.3** (tests assert metrics wiring).
- **8.0.2** after **8.0.1** (docs describe shipped metric names).

## Alternative next cluster (not this wave)

**Coverage depth (`v8.0.0` avoided):** measured bump toward 11%+ — deferred again
while completing the post-pointer operator story (hydrate was optional in 7.0.4).

## References

- Hydrate path: `maidan-bus/src/postgres.rs` (`decode_notify_payload`, `hydrate_envelope`).
- Errors: `maidan-bus/src/error.rs` (`HydrateNotFound`, `HydrateFailed`).
- Listener health pattern: `maidan-bus/src/listener_health.rs`, `maidan-server/src/metrics.rs`.
- Pointer delivery: [[Retros/Cluster 7.0]], [[Decisions]] (NOTIFY pointer entry).
- Subscribe recovery metrics: [[Retros/Cluster 6.0]].
