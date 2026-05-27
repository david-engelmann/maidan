# Cluster 3.0 — Search & subscriber depth

Cluster 2.1 closed OIDC operator hardening at **`v2.1.0`**. This cluster
addresses the highest-value items deferred from [[Retros/Minor 1.2]],
[[Open Work]], and [[Retros/Cluster 2.1]]: semantic search facets,
automatic event-log backfill when subscribers lag, and a CI coverage floor.

> **Goal:** Semantic search respects the same facets as lexical search on
> Postgres; WebSocket and MCP SSE subscribers recover gaps without a manual
> HTTP replay round-trip; CI fails when line coverage drops below a documented
> baseline.
>
> **Target tag:** `v3.0.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| 3.0.1     | `feat(maidan-search): semantic search facets (Postgres)`                | TBD   |
| 3.0.2     | `feat(maidan-server): auto-replay on bus lag (WS + MCP SSE)`           | TBD   |
| 3.0.3     | `ci: coverage minimum gate (llvm-cov fail-under)`                      | TBD   |
| 3.0.retro | `docs(retro): Cluster 3.0 retrospective + v3.0.0 tag prep`            | TBD   |

## Order

1. **3.0.1** — extend `Search::semantic_search` to accept
   [`SearchFilters`](../../crates/maidan-search/src/filters.rs); apply
   `author` / `channel` / `kind` in Postgres SQL; wire HTTP (`mode=semantic`),
   MCP `search_messages`, and OpenAPI; shared integration tests on Postgres
   testcontainers. SQLite remains `Unsupported` for semantic mode.
2. **3.0.2** — on `BusItem::Lagged`, automatically call
   `replay_matching_events` from the subscriber watermark (when
   `filter.workspace_id` is set), then continue live delivery; keep
   `replay_hint` when replay cannot run (no workspace filter). Update
   `ws_subscribe_e2e` and MCP stream tests.
3. **3.0.3** — add `cargo llvm-cov --fail-under-lines` (or equivalent) to
   the coverage job; document baseline and bump policy in [[Operations]].
   Set the threshold from a green `main` measurement in this PR (not a guess).
4. **3.0.retro** + `v3.0.0` tag.

## Exit criteria

- CI green on `main` (all five required checks + coverage gate).
- `GET /workspaces/:wid/search?mode=semantic&author=…` (and channel/kind)
  filters results on Postgres; lexical behavior unchanged.
- WS subscriber that lags receives replayed events from `maidan_events` without
  client-driven `GET …/events` when `filter.workspace_id` is present.
- Coverage job fails when line coverage falls below the recorded baseline.
- [[Retros/README]] includes Cluster 3.0; `v3.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Semantic+facet SQL regressions | Mirror lexical filter binds; Postgres-only integration tests. |
| Auto-replay storms on chronic lag | Cap with existing `REPLAY_LIMIT` (500); log when truncated. |
| Coverage gate flakes on unrelated PRs | Set baseline slightly below measured `main`; bump only in dedicated PRs. |
| MCP/WS diverge on replay | Single helper in `event_stream.rs`; both call sites updated in 3.0.2. |

## Out of scope

- SQLite semantic search (`sqlite-vec` / extension maturity).
- Per-model embedding tables or mixed dimensions.
- Full resumable WS (opaque reconnect tokens beyond `after_id`).
- Codecov upload or per-crate thresholds (artifact upload stays as-is).
- Score normalization across Postgres vs SQLite ranks (Cluster C deferral).

## Dependencies

- **3.0.1** before **3.0.2** is not strict, but ship facets first to keep review focused.
- **3.0.3** is independent; may merge in parallel after **3.0.1** if CI capacity allows.

## References

- Lexical facets: [[Retros/Minor 1.2]] (#123).
- Semantic HTTP mode: [[Retros/Minor 1.3]].
- `replay_hint` today: `maidan-server/src/event_stream.rs`, `ws_subscribe_e2e.rs`.
- Coverage artifact: `.github/workflows/ci.yml` coverage job (Track T.3).
