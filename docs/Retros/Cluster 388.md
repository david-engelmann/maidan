# Cluster 388 retro — Wave 3 #29: CursorTooOld, shapes, Lagged resume

Wave 3 #29 asked for a **fail-loud subscribe cursor** (no silent clamp),
a durable `consumer_id` watermark, projector **shapes**
`{workspace, channel?, thread?, types[]}` with a 409 must-refetch,
`RecvError::Lagged` → resume-from-log + a metric, and a thick SDK
helper (HTTP backfill then WS cutover).

Cluster **389** shipped first (OSS hygiene / `land_gate`) and noted
that **388 was left unused**. This cluster fills that number. Five
impl PRs (388.1–388.5) + this retro. Every PR targets `main` (the
383 `base_ref_deleted` lesson).

## What shipped

- **388.1 (#832) — types + store.** `CursorTooOld`, `cursor_is_too_old`,
  `ProjectorShape`, `parse_projector_types`. Store `min_event_id` /
  `list_events_after_global` / `ensure_cursor_fresh`. HTTP 409
  `https://maidan.dev/problems/cursor-too-old` + `must_refetch: true`.
  Fresh `after_id <= 0` is never too old; adjacent resume
  (`after_id + 1 == oldest`) is fine. **No new table** — Cluster 13/125
  `maidan_delivery_cursor` is the durable `consumer_id` watermark.
- **388.2 (#834) — subscribe wiring.** `ensure_subscribe_cursor` after
  the consumer-id floor on WS, `GET /mcp/stream`, AG-UI, and
  `GET /workspaces/:id/events`. WS sends `{type:"cursor_too_old",
  must_refetch:true}` then closes 1008. Shape query params
  (`channel_id`, `thread_id`, `types`, `consumer_id`); unknown `types`
  → 400. Filtered shapes page until `limit` matches.
- **388.3 (#835) — Lagged resume.** `resume_from_log` pages the global
  log. Webhook, notification router, FSM hooks, AG-UI, and the indexer
  (`with_log`) replay `id > watermark` instead of dropping.
  `maidan_bus_lag_resume_total{consumer,outcome}`. Presence and
  streamable-session `Lagged` stay in-memory — they are not event-log
  drops. Subscribe already auto-replayed (`event_stream`).
- **388.4 (#836) — thick SDK `follow`.** Pages `GET /workspaces/{id}/events`,
  then cuts over to `/ws/subscribe` at the last id. HTTP rows normalize
  to `{log_id, …}`. `is_cursor_too_old` is 409 + `must_refetch` — never
  clamp. `type: cursor_too_old` is delivered, not skipped as a control
  frame. Twins in Python / TypeScript / Go.
- **388.5 (#837) — MCP SSE 409 e2e.** `GET /mcp/stream` with a
  pruned-gap cursor is the same 409 problem document.
- **CI hotfix (on #832–#837).** Lockfile bump `rustls` 0.23.40 →
  **0.23.45** ([RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285)).
  First-party hyper 1.x / reqwest / lettre / tokio-rustls ride 0.23.
  AWS `rustls` 0.21.12 is unaffected (`<0.23.13`). No `deny.toml` ignore.

## Decisions

- **Fail loud, never clamp.** A cursor in a pruned gap is 409
  `must_refetch`. Pretending the remaining log is complete is the
  Postel anti-pattern the item forbade.
- **Reuse `maidan_delivery_cursor`.** A second cursor table would
  duplicate Cluster 13/125. Freshness runs *after* the consumer-id
  floor — a stored watermark can make `after_id=0` too old.
- **409 is CursorTooOld, not shape-mismatch.** A shape that matches
  nothing is an empty page, not a conflict.
- **Internal `Lagged` vs subscribe `Lagged`.** Subscribe already
  auto-replays. The silent-drop bug was webhook / notifications / FSM /
  indexer / AG-UI. Presence and MCP streamable session broadcasts are
  not the event log.
- **`resume_from_log` returns the last id.** Event ids can gap after a
  rolled-back insert; `after + count` is not a safe watermark.
- **SDK is standalone.** `follow` lives in `sdk/` and must not depend
  on `maidan-*` server crates. `after_id` / `consumer_id` are siblings
  of `filter` on the subscribe frame (`additionalProperties: false`).

## Surprises

- **Cluster 389 took the next tag while 388 was reserved.** Titles and
  this retro fill 388; do not renumber 389.
- **The clamp to kill was not `max(requested, cursor)`.** That floor
  is load-bearing. The bug is “cursor in a pruned gap → pretend the
  remaining log is complete.”
- **Adding an EventKind still needs the federatable / ALL / contract
  drill** — not needed here; no new kind.
- **RUSTSEC-2026-0285 landed in the advisory DB after 389's last green
  merge.** `cargo deny` is a function of time. Upgrade the first-party
  0.23 line; do not ignore a patched crate.

## Test evidence

- Types: `cursor_is_too_old` (fresh / empty / adjacent / gap) +
  `ProjectorShape` match / parse-fails-loud.
- Store: `cursor_too_old` + `lag_resume` (both backends; Postgres
  testcontainers skip without Docker).
- Server: `cursor_too_old_subscribe_e2e` — REST 409, shape `types`,
  durable consumer_id 409, WS frame+1008, MCP SSE 409.
- SDK: rust `cargo test --lib` (normalize / control-frame /
  `is_cursor_too_old`); Go `IsCursorTooOld`; Python / TS unit twins.
  Black-box `scripts/sdk-test.sh` still needs `MAIDAN_URL`.
- `cargo deny check` green after rustls 0.23.45. Clippy `-D warnings`
  on types/store/search/server.

## Forward look

**Cluster 388 is complete. Row #29 is closed.**

Do not start Wave 3 #30–36 from this retro. #30 is the EventKind
JSON-Schema pack + `$type` evolution + a projector-lag header — a
different item.

Deferred: persist the in-process Lagged watermark across
`StreamEnded`; a live `sdk-test.sh` follow round-trip; shape-keyed
409 (declined).

## Acknowledgements

#832 foundation → #834 subscribe → #835 Lagged → #836 SDK → #837 MCP
SSE + this retro. 389 agents kept OSS hygiene; we filled the unused
number.
