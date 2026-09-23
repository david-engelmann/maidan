# Cluster 407 retro — executable surface and hero-loop contracts

> Wave 4 row #39 · `v407.0.0` · umbrella #963 · PRs #965/#966/#967/#968 + close record

## Outcome

Four previously informal compatibility claims are now executable. The no-build
browser console cannot silently drift from its session proxy or OpenAPI; every
event kind has an intentional, evidenced producer-surface disposition; a real
signed browser session completes one causally ordered HTTP-to-WebSocket
collaboration loop; and the portable export, snapshot, and catch-up wire shapes
are reviewable as normalized JSON.

| Slice | Evidence | Result |
|-------|----------|--------|
| 407.1 | #965; `ui_fetch_openapi_contract`; HTTP capability-map contracts | UI fetch templates, mounted proxy methods, and OpenAPI operations are derived from their owning sources and compared. The first run added four omitted group-DM operations to OpenAPI and the capability map. |
| 407.2 | #966; `event_surface_disposition_contract`; full MCP suite | All 29 `EventKind` values are classified as REST-only, MCP-only, both, or internal-only with executable evidence. MCP artifact uploads now atomically append and publish `ArtifactUpserted`. |
| 407.3 | signed-session `ui_channels_e2e`; strict clippy | One session cookie subscribes, waits for acknowledgement, writes channel/thread/message state through `/ui/api`, observes the exact durable live event, reads it back, and cannot spoof another author. |
| 407.4 | `portable_frame_goldens`; `scripts/check-agent-contract.sh` | Normalized signed-export and snapshot/catch-up fixtures lock protocol types, JSON types, relationships, arrays, LSNs, enums, and field presence without freezing entropy. |

## Decisions

- The UI contract derives three independent views from the embedded JavaScript,
  Axum router, and generated OpenAPI. There is no fourth hand-maintained route
  manifest that can agree with itself while production drifts.
- Event parity is a disposition, not a demand for symmetry. Worker-derived facts
  remain internal-only, workspace/bootstrap administration remains REST-only,
  and agents remain the requester side of durable approvals over MCP.
- The hero test waits for `subscribe_ack` before issuing the write. Arrival after
  an unacknowledged subscription would be timing luck rather than causal proof.
- Goldens preserve semantic types and identifier relationships. Generated UUIDs,
  wall-clock values, hashes, keys, and signatures are named placeholders rather
  than accidental compatibility promises.

## What surprised us

- OpenAPI and the HTTP capability map agreed because both omitted the same four
  live group-DM routes. Comparing two inventories with a shared blind spot had
  reported false confidence.
- MCP artifact upload appeared equivalent to REST but used the non-eventing
  metadata upsert and recorded the workspace ref separately. No
  `ArtifactUpserted` reached the durable log or live bus.
- Signed-session HTTP writes and signed-session WebSocket authentication each
  had coverage, but in separate tests. Nothing proved the browser's own write
  reached its already-established live tail.
- Anonymous redaction would have made the fixtures smaller but hidden whether
  workspace/member/channel/thread/message references still agreed. Named
  placeholders made relational drift visible.

## Residual risk and follow-up

- The embedded-JavaScript census is a small balanced-call parser, not a full
  JavaScript AST. It deliberately fails closed on malformed calls and is paired
  with Playwright, but a future syntax rewrite may require replacing the parser.
- Event evidence validates an exact test function and its surface classification;
  it cannot introspect the assertions inside that test. Review remains responsible
  for rejecting evidence that names a test without proving the claimed event.
- The portable fixtures are representative typed frames, not a byte capture from
  a deployed database. Existing endpoint e2e tests cover transport behavior; the
  goldens cover stable serialization and compatibility review.
- Browser layout and screenshots remain visual/product evidence, not required
  protocol gates. The existing Playwright job continues to own rendered behavior.

## Release ledger

| Item | Value |
|------|-------|
| Tag | `v407.0.0` |
| Roadmap | Wave 4 row #39 closed |
| Database compatibility | No migration |
| API compatibility | Four already-live group-DM operations documented; no route removed |
| Runtime behavior | MCP artifact uploads now emit the same durable event as REST |
| New release gates | UI/OpenAPI census, event disposition, signed-session live loop, normalized portable-frame goldens |
