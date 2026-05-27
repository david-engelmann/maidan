# Cluster 4.0 — Subscriber continuity

Cluster 3.0 shipped semantic search facets, auto-replay on bus lag, and a CI
coverage floor at **`v3.0.0`**. Subscribers can recover gaps without a manual
HTTP round-trip when `filter.workspace_id` is set, but reconnecting still
requires the client to remember `after_id` and the full filter JSON.

> **Goal:** WS and MCP SSE clients can resume after disconnect with an opaque
> server-issued token, and learn when auto-replay truncated so they can continue
> from the advertised watermark without guessing.
>
> **Target tag:** `v4.0.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| 4.0.1     | `feat(maidan-server): signed subscribe resume tokens (WS + MCP SSE)`   | #151  |
| 4.0.2     | `feat(maidan-server): replay_truncated frame after capped auto-replay` | #152  |
| 4.0.3     | `feat(maidan-server): OpenAPI + docs for subscribe resume protocol`    | #153  |
| 4.0.retro | `docs(retro): Cluster 4.0 retrospective + v4.0.0 tag prep`            | TBD   |

## Order

1. **4.0.1** — on successful subscribe (WS first text frame, MCP stream connect),
   optionally accept `resume_token` instead of `filter` + `after_id`. When
   starting fresh, return a `subscribe_ack` frame containing a signed
   `resume_token` encoding `filter`, watermark (`after_id` / last `log_id`),
   and expiry (HMAC with `MAIDAN_SESSION_SECRET` or a dedicated
   `MAIDAN_SUBSCRIBE_RESUME_SECRET`). Reconnect passes only the token + bearer
   auth. Invalid/expired/tampered tokens close with `1008`.
2. **4.0.2** — when `replay_matching_events` returns exactly `REPLAY_LIMIT` rows,
   emit a `replay_truncated` frame (`after_id`, `limit`, optional
   `workspace_id`) before resuming live bus delivery so clients can loop without
   relying on `replay_hint` alone. Apply in shared `event_stream.rs` (WS + MCP).
3. **4.0.3** — document subscribe/resume in [[Production]], extend OpenAPI
   where applicable, add `ws_subscribe_e2e` resume round-trip and truncation
   tests. Update [[Architecture]] subscriber diagram.
4. **4.0.retro** + `v4.0.0` tag.

## Exit criteria

- CI green on `main` (including coverage gate from 3.0.3).
- WS client can disconnect and reconnect with `resume_token` only; receives
  events with `log_id` strictly greater than the stored watermark.
- When auto-replay fills `REPLAY_LIMIT`, client receives `replay_truncated` and
  can continue with the advertised `after_id`.
- MCP SSE exposes the same ack/truncation frames on `/mcp/stream`.
- [[Retros/README]] includes Cluster 4.0; `v4.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| Token size on the wire | Keep filter small; sign compact JSON; reject oversized frames. |
| Stale resume after filter change | Short TTL (e.g. 1h); token binds to workspace + filter snapshot. |
| Token forgery | HMAC with server secret; same pattern as session cookies (2.1.1). |
| Duplicate events on resume | Watermark is exclusive (`log_id > after_id`); bus path unchanged. |
| MCP/WS divergence | Shared helpers in `event_stream.rs` for ack + truncation. |

## Out of scope

- Postgres `LISTEN`/`NOTIFY` at-most-once semantics (standing risk; not fixable in-server alone).
- Server-side subscribe session table (opaque signed token only for this cluster).
- Per-model embedding tables / SQLite semantic search (see [[Open Work]]).
- Codecov upload or raising the global coverage % floor (candidate **Cluster 5**).
- SSE for MCP `resources/subscribe` (Cluster B deferral).

## Alternative next cluster (not this wave)

**Cluster 5 — Coverage & search quality** (`v5.0.0`): raise `COVERAGE_MIN_LINES`
with targeted tests, optional Codecov, embedding model metadata, score
normalization across backends.

## References

- Auto-replay on lag: [[Retros/Cluster 3.0]] (#147), `event_stream.rs`.
- `after_id` resume today: `ws_subscribe_e2e.rs` (`subscribe_resumes_after_id_from_event_log`).
- `REPLAY_LIMIT`: `maidan-server/src/event_stream.rs` (`500`).
