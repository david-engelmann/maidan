# Cluster 166.0 — perf/correctness: SQLite pragmas + webhook fan-out

**Theme:** First cluster of **Arc 2 (perf + CI/CD)** — the two highest-value
DB fixes from the performance research (a real SQLite correctness bug + the
biggest per-event query win).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v166.0.0`**, no new gate tag.

---

## Scope

| Fix | Where |
|-----|-------|
| SQLite `foreign_keys`/`busy_timeout`/WAL applied per connection (R3) | `crates/maidan-search/src/sqlite_vec.rs` (`pool_options_with`), `main.rs` |
| Webhook fan-out queries only the event's workspace (H1) | `webhook_worker.rs` + store `*_for_workspace` (both backends) |

## Why

- **R3:** `configure_pool` ran the PRAGMAs via `.execute(pool)` — one pooled
  connection. `foreign_keys` and `busy_timeout` are **per-connection** in SQLite,
  so every *other* pooled connection ran with **FK enforcement off** (a real
  data-integrity risk) and fail-fast-on-`SQLITE_BUSY`. Moving them into the
  pool's `after_connect` hook applies them to every connection.
- **H1:** `enqueue_matches` called `list_enabled_webhook_subscriptions()` —
  **all** enabled subs across **all** workspaces — on **every** bus event, then
  filtered in memory, and built the payload unconditionally. Now it queries only
  the event's workspace (indexed) and builds the payload lazily on first match.

## Non-goals

- The rest of Arc 2's perf items (H6, H4, R2, H2, R1) — next cluster.
- **CI/CD workflow speedups** (arm64 runner, build-once image, gha cache, trivy)
  — deferred until GitHub Actions recovers; they only run in Actions and can't be
  validated during the current outage.

## Exit criteria

- FK + busy_timeout on every pooled connection; webhook fan-out scoped to the
  event's workspace; webhook e2e unchanged — **met**.
- `v166.0.0` tagged.

## Verification & limits

- `pragmas_apply_to_every_pooled_connection` (file-backed, 3 held connections):
  each has `foreign_keys = 1` and the configured `busy_timeout`. Webhook +
  mention-webhook e2e green; full store suite green.
- **CI note:** GitHub Actions outage — validated locally; re-run CI on `main`
  when recovered.

## References

- [[Retros/Cluster 166.0]]; `sqlite_vec.rs`, `webhook_worker.rs`,
  `{postgres,sqlite}/webhooks.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program`.
