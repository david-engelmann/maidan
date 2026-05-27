# Cluster 4.0 retro — Subscriber continuity

> Closing wave for Cluster 4.0 · target tag `v4.0.0`.

Cluster 4.0 closed the reconnect gap left after `v3.0.0`: opaque resume tokens,
explicit truncation signaling when replay fills one window, and operator docs for
the subscribe protocol.

## What shipped

- **PR #151** — HMAC `resume_token` + `subscribe_ack` on WS and MCP SSE; compose
  smoke `MAIDAN_SESSION_SECRET` for the full profile.
- **PR #152** — `replay_truncated` control frame when event-log replay returns
  `REPLAY_LIMIT` (500) rows (subscribe replay, bus-lag auto-replay).
- **PR #153** — Production/Architecture subscribe docs, OpenAPI `info.description`,
  truncation e2e (`subscribe_emits_replay_truncated_when_event_log_exceeds_replay_limit`).

## What was deferred

| To           | What                                              | Why                                      |
|--------------|---------------------------------------------------|------------------------------------------|
| Cluster 5    | Raise `COVERAGE_MIN_LINES` + Codecov              | Separate CI/coverage cluster.            |
| Cluster 5    | Per-model embedding tables / SQLite semantic      | Search-quality scope.                    |
| Post-4.0     | Server-side subscribe session table               | Signed token sufficient for this wave.   |
| Cluster B    | SSE for MCP `resources/subscribe`                 | Long-standing deferral.                  |

## Surprises

- Full-profile compose smoke failed until `MAIDAN_SESSION_SECRET` was set — resume
  signing runs whenever auth is on, not only under OIDC.
- Resume reconnect with `after_id=0` in the token still needs an explicit replay
  pass (`from_resume_token`) or offline events are missed.
- Truncation is keyed off the raw store row count (`500`), not post-filter envelope count.

## Decisions

- **Opaque resume token** — HMAC JSON payload (`filter`, `after_id`, `exp`); no
  server-side session table. [[Architecture]] updated with sequence diagram.
- **Truncation signal** — `replay_truncated` with advertised `after_id` watermark;
  clients loop until the frame stops appearing.
- **OpenAPI scope** — subscribe protocol documented in `info.description`; dedicated
  WS schemas remain future work.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Signed WS/MCP `resume_token` + `subscribe_ack`          | `v4.0.0`           |
| `replay_truncated` after capped event-log replay        | `v4.0.0`           |
| Subscribe/resume operator docs + truncation e2e         | `v4.0.0`           |

## Risks identified + mitigated

- **Reconnect requires remembering filter + after_id** — resume token encodes both.
- **Unknown replay incompleteness** — `replay_truncated` advertises continuation watermark.

## Risks identified + still open

- **At-most-once bus delivery** — Postgres `LISTEN`/`NOTIFY` unchanged; replay paths
  remain best-effort gap fill.
- **Stale resume after filter change** — mitigated by TTL; clients should prefer latest
  `subscribe_ack`.
- **Coverage depth** — floor from `v3.0.0` unchanged.

## Forward look

**Cluster 5** (candidate): coverage uplift, Codecov, embedding model metadata, score
normalization — see [[Open Work]] and [[Clusters/Cluster 4.0#Alternative next cluster]].

## Acknowledgements

Solo cluster. Three implementation PRs plus this retro.
