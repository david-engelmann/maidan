# Cluster 6.0 — Delivery reliability

Cluster 5.0 closed coverage and search quality at **`v5.0.0`**. Subscribers can
resume and loop on truncation, but operators still infer delivery health from
logs: bus lag triggers auto-replay or `replay_hint`, indexer staleness requires
opt-in `INDEXER_STALE_SECS`, and Prometheus today only covers HTTP — not
subscribe recovery paths.

> **Goal:** Make gap/recovery visible in metrics and readiness guidance; document
> how to alert on bus lag, replay truncation, indexer silence, and listener errors
> without changing at-most-once bus semantics.
>
> **Target tag:** `v6.0.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| 6.0.1     | `feat(maidan-server): Prometheus metrics for subscribe replay paths`   | TBD   |
| 6.0.2     | `feat(maidan-server): indexer age metric + production stale defaults`  | TBD   |
| 6.0.3     | `feat(maidan-bus): listener health Prometheus gauges`                   | TBD   |
| 6.0.4     | `docs: delivery reliability runbook + alerting on subscribe metrics`   | TBD   |
| 6.0.retro | `docs(retro): Cluster 6.0 retrospective + v6.0.0 tag prep`            | TBD   |

## Order

1. **6.0.1** — increment counters in shared `event_stream.rs` (WS + MCP SSE):
   - `maidan_subscribe_replay_total` — labels `transport` (`ws` | `mcp_sse`),
     `outcome` (`auto_replay` | `replay_hint` | `replay_truncated` |
     `auto_replay_failed`).
   - `maidan_bus_lag_total` — label `transport`; record `skipped` from
     `BusItem::Lagged` (histogram or counter with `le` buckets TBD in impl).
   - Wire `metrics::init()` descriptions; extend `metrics_e2e` or
     `ws_subscribe_e2e` to assert counters move on lag/truncation paths.
2. **6.0.2** — expose `maidan_indexer_last_event_age_seconds` gauge (0 when never
   seen); document recommended `INDEXER_STALE_SECS` when embeddings are enabled
   (e.g. full `compose.yaml` profile, [[Production]]). Keep default `0` for dev
   so local runs stay quiet; readiness behavior unchanged unless env set.
3. **6.0.3** — export Postgres `ListenerHealth` as Prometheus gauge
   `maidan_bus_listener_ok` (1/0) on servers using `PostgresBus`; optional
   `maidan_bus_listener_retries_total` if cheap to instrument in the listener loop.
4. **6.0.4** — [[Production]] alerting table (metric → symptom → action);
   [[Operations]] subscribe-troubleshooting section; [[Architecture]] subscriber
   ops diagram referencing metrics. OpenAPI note on `/metrics` cardinality.
5. **6.0.retro** + `v6.0.0` tag.

## Exit criteria

- CI green on `main` (five required checks + coverage floor from 5.0).
- `/metrics` exposes subscribe replay and bus-lag series; e2e proves at least one
  counter increments on a lag or truncation path.
- Indexer age gauge present; production docs recommend `INDEXER_STALE_SECS` when
  embeddings enabled.
- Postgres deployments expose bus listener health on `/metrics`.
- [[Retros/README]] includes Cluster 6.0; `v6.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Metric cardinality explosion | Fixed label sets only; no workspace UUID labels. |
| Flaky e2e on counter values | Assert `>= 1` after known scenario, not exact totals. |
| `INDEXER_STALE_SECS` breaks dev | Default remains `0`; examples opt in. |
| Duplicating health JSON in metrics | Metrics complement `/health`; do not remove readiness fields. |

## Out of scope

- Postgres `LISTEN`/`NOTIFY` at-most-once fix (standing risk; needs design beyond metrics).
- Server-side subscribe session table (4.0 chose signed tokens).
- Coverage floor bump to 11%+ (separate incremental PR if desired).
- SQLite semantic search, per-model embedding tables.
- SSE for MCP `resources/subscribe` (Cluster B deferral).

## Dependencies

- **6.0.1** before **6.0.4** (docs reference metric names).
- **6.0.2** and **6.0.3** independent of each other; either may merge before 6.0.1.
- **6.0.4** after implementation PRs land (or documents planned names if parallel).

## Alternative next cluster (not this wave)

**Incremental coverage** (`v6.0.0` avoided): measured bump toward 11%+ with targeted
tests — lower operator value than delivery observability per [[Open Work]].

## References

- Auto-replay + truncation: [[Retros/Cluster 3.0]], [[Retros/Cluster 4.0]],
  `maidan-server/src/event_stream.rs`.
- Bus listener health: [[Retros/Minor 1.1]], `maidan-bus/src/listener_health.rs`.
- Indexer staleness: Track T.2, `maidan-server/src/health.rs`.
- HTTP metrics baseline: `maidan-server/src/metrics.rs` (Track T.4).
