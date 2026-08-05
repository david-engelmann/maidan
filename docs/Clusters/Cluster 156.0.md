# Cluster 156.0 — production-safety defaults (SIGTERM drain + statement timeout)

**Theme:** First cluster of the **enterprise-hardening arc** (arc 1 of the
post-v155 program). Two safe-by-default fixes the production-readiness research
flagged as quick wins.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v156.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Drain on `SIGTERM` as well as `SIGINT` | `crates/maidan-server/src/main.rs` |
| Default `MAIDAN_DB_STATEMENT_TIMEOUT_MS` `0` → `30000` | `crates/maidan-server/src/config.rs` |
| Doc the new default + caveat | `docs/Production.md` |

## Why

- **SIGTERM:** k8s and systemd send `SIGTERM` on rollout/stop. The server only
  waited on `ctrl_c()` (SIGINT), so pods were SIGKILL'd after the grace period
  instead of draining through the existing `with_graceful_shutdown` + worker
  `shutdown()` sequence — dropping in-flight requests/events on every deploy.
- **statement_timeout:** defaulting to `0` (disabled) let a single runaway query
  hold a pooled connection indefinitely — a noisy-neighbor / DoS vector on a
  shared multi-tenant instance. A 30 s per-statement cap is far above any
  healthy query; migrations are already exempt.

## Non-goals

- Auth fail-closed, default-on rate limits, body-size cap — each a separate
  arc-1 cluster (they touch the required smoke jobs / artifact upload and need
  coordinated changes).

## PR ladder (actual)

| # | Title |
|---|--------|
| 156.0.1 | `feat(server): SIGTERM graceful shutdown + default statement_timeout` (#402) |
| 156.0.retro | `docs(retro): Cluster 156.0 + v156.0.0 tag prep` |

## Exit criteria

- SIGTERM drains gracefully; `statement_timeout` defaults to 30 s and is
  disable-able; tests green — **met**.
- `v156.0.0` tagged after retro.

## Verification & limits

- Unit: `db_config_defaults_are_safe`, `db_config_statement_timeout_can_be_disabled`.
- SIGTERM handling is signal wiring in `main` (not unit-tested, per codebase
  norm); it compiles and reuses the tested graceful-shutdown drain. Migrations
  reset `statement_timeout = 0` under the advisory lock, so the new default can't
  break startup or a rolling migration.

## References

- [[Retros/Cluster 156.0]]; `main.rs` (shutdown), `config.rs` (`DbConfig`),
  [[Production]] DB tuning. Program: [[Roadmap]] + memory `maidan-next-arc-program`.
