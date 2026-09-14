# Cluster 390 retro — Wave 3 #30: EventKind lexicon, `$type`, Room-LSN

Wave 3 #30 asked for an **EventKind JSON-Schema pack** (lexicon analogue,
feeds a future SDK 0.2), **`$type` evolution** (Hyrum: the observable
`$type` is the contract), a **`Maidan-Room-LSN`** header on
REST/WS/MCP/A2A/projectors so clients see projector / broadcast lag, and
canon **NEW-snapshot-tests** over the normalized shapes.

This is **not** `Maidan-Consistency-Token` (Cluster 263): that header is
a Postgres WAL LSN, replica-gated, and answers read-your-writes. Room-LSN
is the event-log high-water (`MAX(maidan_events.id)`, `0` if empty),
always on (SQLite too), decimal, and answers "how far behind is my
projector?"

Four impl PRs (390.1–390.4) + this retro. Every PR targets `main`.
**Row #30 is closed.** Do not start Wave 3 #31–36 from this close.

## What shipped

- **390.1 (#839) — lexicon pack + `$type`.** `EventKind::type_id` /
  `parse_type_id` (`maidan.event.{snake}/1`; `/2` is a different type).
  `crates/maidan-types/src/lexicon.rs`: `inject_type`, `event_wire`,
  `normalize`, `pack_files`, `catalog`. Committed pack under
  `contracts/lexicon/` (28 event schemas + `maidan.waiter.result/1` +
  generic `example.review.result/1` / `example.plan.result/1`). Snapshot
  tests over normalized wire shapes. Stored `maidan_events.payload`
  still tags on `kind`.
- **390.2 (#840) — room head.** `RoomLsn` + `ROOM_LSN_HEADER`.
  `Store::max_event_id` on both backends (Postgres reused the Cluster-258
  bus helper; SQLite gained the twin). Parse **rejects `/`** so a WAL
  token cannot be read as a room head.
- **390.3 (#841) — header + live `$type`.** Always-on middleware stamps
  `Maidan-Room-LSN` after the handler (skips `/health*`, `/metrics`,
  `/openapi.json`, `/ui`, `/.well-known/`). WS `subscribe_ack.room_lsn`
  is load-bearing (many clients miss 101 headers). Live WS/MCP frames
  get `$type` (full + lean). A2A RPC is covered by the outer router.
- **390.4 (#842) — projectors + SDKs.** Webhook `build_payload` injects
  `$type`; `deliver_http` stamps Room-LSN (automation deliveries too).
  Slack/GitHub API egress is not stamped. Four clients capture
  `last_room_lsn` / `LastRoomLSN` / `lastRoomLsn` and reject WAL text.
  `event_type` helper. SDK stays **0.1.0**.

## Decisions

- **`$type` is a wire envelope, not a stored column.** Serde still
  tags `Event` on `kind`. Injecting on the stored payload would rewrite
  history and break read-back. Hyrum's home is what a subscriber sees.
- **Evolution rules are the pack's contract.** New fields optional, no
  renames, unknown ignored, breaking = new type (`/2`).
- **Two tokens, two names.** Do not parse `Maidan-Room-LSN` as a WAL
  LSN or echo it as `Maidan-Consistency-Token`. Different value space,
  gating, and purpose.
- **WS ack carries `room_lsn`.** Upgrade response headers are not
  reliable across WS clients.
- **Generic namespaced extras only.** Waiter envelope
  `maidan.waiter.result/1`; examples `example.review.result/1` and
  `example.plan.result/1`. No product-specific type ids.
- **No migration.** Room-LSN is `MAX(id)` on the existing event log.
- **SDK 0.2 is still typed models.** The pack is the input; this
  cluster does not bump the published 0.1.0 clients.

## Surprises

- **REST `GET /events` stays `StoredEvent` without `$type`.** Live
  frames and webhook bodies carry it. Wrapping the OpenAPI list would
  have been a contract bump in the same cluster as the header.
- **`after_id > 0` still requires `filter.workspace_id`.** Room-LSN
  e2e replay must set both or the socket closes 1008.
- **Webhook HMAC covers `$type`.** Inject before `sign_payload`, not
  after.

## Test evidence

- Types: lexicon snapshots + `EventKind::type_id` / `parse_type_id`
  (`/2` inert) + waiter `$type` alias of `schema`.
- Store: `room_lsn` both backends (Postgres testcontainers skip
  without Docker).
- Server: `room_lsn_e2e` (REST decimal, no consistency token on
  SQLite, `/health/live` unstamped, WS ack + `$type`, MCP SSE, A2A
  RPC); `webhooks_e2e` Room-LSN + `$type` on the signed POST;
  `build_payload_stamps_type_on_envelope_and_event`.
- SDK: rust `parse_room_lsn` rejects WAL; Python / Go / TS twins.

## Forward look

**Cluster 390 is complete. Row #30 is closed.**

Do not start Wave 3 #31–36 from this retro. #31 is a signed workspace
export a blank instance can verify.

Deferred: `$type` on REST `GET /events` OpenAPI `StoredEvent`; Slack /
GitHub API egress Room-LSN; SDK 0.2 codegen from `contracts/lexicon/`;
streaming workspace export (already Open Work elsewhere).

## Acknowledgements

#839 lexicon → #840 store head → #841 header + live frames → #842
webhooks + four SDKs → this retro (#843).
