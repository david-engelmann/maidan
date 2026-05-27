# Cluster 13.0 — Delivery contract & subscriber ledger

Cluster 12.0 closed outbox relay hardening at **`v12.0.0`**. Subscribers already dedupe
by `log_id` and use replay HTTP / WS `resume_token`, but the server does not persist
per-consumer delivery cursors. Federation pull and long-lived WS/SSE sessions would
benefit from an explicit **delivery ledger** and documented idempotency contract.

> **Goal:** Persist durable delivery cursors for at-least-once fan-out; document
> subscriber idempotency; optional server-side skip of already-delivered `log_id`s
> for registered consumers — without claiming exactly-once end-to-end.
>
> **Target tag:** `v13.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 13.0 kickoff plan`                                      | TBD   |
| 13.0.1     | `feat(maidan-store): delivery cursor schema + store API`               | TBD   |
| 13.0.2     | `feat(maidan-server): federation pull worker uses cursor`              | TBD   |
| 13.0.3     | `feat(maidan-server): optional WS/SSE consumer_id + cursor advance`    | TBD   |
| 13.0.4     | `docs: delivery contract in Decisions/Architecture/Production`       | TBD   |
| 13.0.retro | `docs(retro): Cluster 13.0 retrospective + v13.0.0 tag prep`          | TBD   |

## Order

1. **13.0.1** — migration `maidan_delivery_cursor` (or similar):
   `(consumer_id, workspace_id)` → `last_delivered_log_id`, `updated_at`.
   Store trait + Postgres impl: `get_cursor`, `advance_cursor` (monotonic `log_id` only).
2. **13.0.2** — federation outbound/inbound path advances cursor after successful
   handoff (smallest scope: federation pull worker only).
3. **13.0.3** — optional `consumer_id` on WS/MCP subscribe (header or query); on
   each delivered event, advance cursor; on subscribe, skip replay below cursor when
   `consumer_id` matches persisted row.
4. **13.0.4** — document idempotency contract (`log_id` primary key); cursor semantics;
   interaction with outbox quarantine and NOTIFY duplicates.
5. **13.0.retro** + `v13.0.0` tag.

## Exit criteria

- CI green on `main`.
- Federation path (or documented first consumer) persists and respects cursors.
- Docs state at-least-once + `log_id` dedup + cursor behavior.
- `v13.0.0` tagged after retro.

## Risks

| Risk | Mitigation |
|------|------------|
| Cursor regression hides events | Monotonic advance only; compare `log_id` ordering |
| Scope creep (all transports) | Ship federation first; WS/SSE optional in 13.0.3 |
| Exactly-once expectations | Explicit docs: cursor reduces duplicates, not guarantees |

## Out of scope

- NOTIFY guaranteed delivery.
- SQLite delivery cursors (Postgres first).
- Outbox replay HTTP API.
- Full exactly-once across federation boundaries.

## Follow-on

| Cluster | Tag | Theme |
|---------|-----|--------|
| **14.0** | `v14.0.0` | Epic pick (SQLite semantic, SQLite outbox, MCP subscribe, S3 multipart) |

## References

- Cluster 10–12 outbox/delivery: [[Retros/Cluster 10.0]], [[Retros/Cluster 12.0]].
- Federation: `maidan-server/src/federation.rs`, Cluster G retro.
- Subscribe: `event_stream.rs`, `subscribe_resume.rs`.
