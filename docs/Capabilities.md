# Capabilities

A running list of what Maidan can do, by release. Each cluster's retro
PR prepends a new section so the latest is always at the top.

## v399.0.0 — the WASI slash-handler runtime (Wave 3 #36)

Four PRs (399.1–399.4). Cluster 396 left `SlashHandlerKind::wasi` registrable
with no runtime behind it — every dispatch returned `wasi_runtime_unavailable`,
so a workspace could successfully configure a handler that could never run.
**Row #36, and with it Wave 3, is now closed.**

| Change | Where |
|--------|-------|
| **The sandbox (399.1):** a `wasmi` host — an interpreter, so there is no code generator in the trust path and no miscompilation-to-escape class. Fuel metering, a linear-memory cap via `ResourceLimiter`, and an output cap. Imports are checked against the allowlist *before* instantiation, so a banned import is named rather than surfacing as a generic link failure. `wasmi_wasi` was dropped: it pulls a duplicate `wast` that `cargo deny` bans, and implementing the 16 preview-1 calls directly is the better design — the host implements those and only those, so there is nothing to filter and nothing to escape from. | `crates/maidan-wasi/` |
| **Module fetch + tenancy (399.2):** `handler_target` is a sha256, so a module lives in the content-addressed artifact store and the Cluster-204 access link is what makes it belong to a workspace. Checked against `command.workspace_id` — a handler is installed by the workspace and runs for anyone in it. Without it, a registration could name any sha on the instance and Maidan would *execute* it. The guest runs on `spawn_blocking`: interpreting wasm is synchronous and CPU-bound, and on the async runtime a few slow handlers would stall unrelated requests. | `crates/maidan-server/src/wasi_handler.rs` |
| **Failure semantics (399.3):** fuel, memory, trap and non-zero exit each report their own kind. A `proc_exit(3)` was previously classified `Trap` with its message overwritten — the kind and the prose disagreed, and the kind is what a caller branches on. Precedence is now a decision: a host-enforced limit outranks the guest's own verdict, because a guest that hits the memory cap typically fails its next allocation and *then* exits from its own error path. Failure text is bounded in the one constructor every failure goes through (a wasm validation error quotes an attacker-chosen import name). | `crates/maidan-types/src/wasi.rs`, `crates/maidan-wasi/src/lib.rs` |
| **Two output bounds (399.3):** the sandbox's 256 KiB cap protects host memory during a run; a slash response is persisted into the triggering message's metadata and fanned out to every subscriber, so the room gets its own 16 KiB bound. Both cuts are marked — a clipped value never reads as a complete one. | `crates/maidan-server/src/wasi_handler.rs` |
| **Registration verifies ownership (399.4):** the sha must name an artifact the workspace owns, on both write surfaces, or the command is refused. Registration asks "can this ever run?"; dispatch keeps asking "may this run *now*?", because a workspace can lose an artifact afterwards. Both refusals return one message so registration is not an oracle for what exists on the instance. | `crates/maidan-auth/src/access.rs`, REST + MCP |
| **The docs it never had (399.4):** build → upload → register, the invoke/result ABI, the allowlist and why it is structural, both bounds, and each failure kind mapped to what its author should do. Writing them found two things: the crate doc claimed 17 allowlisted calls (there are 16), and **nothing checked that an allowlisted name is actually registered** — a name on the list but not implemented passes the import check and then dies in the linker as `invalid_module`, which reads as "your wasm is broken" to the one person who did nothing wrong. | [WASI-Handlers.md](WASI-Handlers.md), `Integration.md` |

Wall-clock is deliberately absent from the failure vocabulary: no allowlisted
import blocks, so a guest cannot wait, and fuel bounds compute well under the
dispatch timeout. A kind that never fires would be a lie.

## v398.0.0 — verification sweep: what was built, tested, and never wired

Eight PRs (398.1–398.8) — three more than this section originally recorded.
Where 397 answered an audit's findings, 398 asked a
different question — *is anything claimed but not actually done?* — and answered
it by enumerating the `Store` trait's 409 methods and reading what had no caller
outside the store crate. Most of the residue is benign (methods superseded by
their `*_with_event` twins in the 205–214 outbox migration); the rest is below.
It is the only check that finds "built, tested, never wired", because there is no
caller to fail a test.

| Change | Where |
|--------|-------|
| **Outbox claim (398.1):** the relay is spawned in *every* replica and `validate_startup` refuses to disable it in production, so an unlocked `list_pending` relayed every row once per replica — N POSTs to each tenant webhook (no unique on `(subscription_id, log_id)`) and N `fsm_hook` firings under `AuthContext::bypass()`. Now a leased `claim_pending` (pg `FOR UPDATE SKIP LOCKED`, sqlite serialized-writer), released on failure, reclaimable after the lease. A claim is not a publish — at-least-once holds. | `crates/maidan-store/src/{postgres,sqlite}/outbox.rs`, `crates/maidan-server/src/outbox_relay.rs`, pg 0095 / sqlite 0094 |
| **DLQ visibility (398.2):** `count_dead_egress` / `count_dead_mail` existed on both backends with **zero callers** — the egress and mail dead-letter queues had no gauge and no alert, while the outbox has had both since Cluster 90. A projector delivery that gave up on a tenant's Slack channel, or an email that would never arrive, accumulated silently. | `crates/maidan-server/src/metrics.rs`, `docs/alerts/prometheus-rules-maidan-slo.yaml` |
| **Panic on a getter (398.2):** `subscribe_resume_secret()` panicked on its `None` arm. Unreachable via `main.rs` (all four branches set a secret or refuse boot) but reachable through the library API, where `AppState::new` leaves it `None`. Now `Option<&[u8]>`; six call sites degrade to a `500` / close code. | `crates/maidan-server/src/{state,ws,mcp_stream}.rs`, `routes/approval_gate.rs` |
| **Mail DLQ scope + `operator:global` (398.3):** `GET /operator/mail/dead` ran a global query behind the per-workspace `token:admin`, and the rows carry `to_address`, `subject` and the body — any workspace admin could read every tenant's outbound email and requeue it to their recipient. `maidan_mail_outbox` gains `workspace_id`; both routes scope to the caller. New `operator:global` capability (in `maidan.human.admin`, never in `maidan.agent.worker`) covers the `NULL`-workspace rows and `GET /operator/legal-holds`, which is genuinely instance-wide. `audit:read-global` could not be reused — it is a *read* capability and the requeue is a write. | `crates/maidan-auth/src/capability{,_set}.rs`, `crates/maidan-server/src/routes/{mail_ops,workspace}.rs`, pg 0096 / sqlite 0095 |
| **Argument strictness (398.4–398.5):** of 128 MCP `*Args` structs — none with `deny_unknown_fields`, 47 with `#[serde(default)]` — exactly two have the property that makes strictness matter: **omission is load-bearing, and its meaning is "turn a control off."** `SetBudgetArgs` (a typo'd dimension silently removes that limit; the replace itself is documented and correct) and `RequestApprovalArgs` (a typo'd `thread_id` leaves the gate unattached, so `claim_next` never blocks and the agent proceeds without the human). The other 45 were left alone deliberately at this point — **a decision 398.6 then reversed.** `ImportArgs` was checked and **excluded**: its deserialize error is swallowed by an `Err(_)` fallback, so the attribute would be dead code, and it fails safe. | `crates/maidan-types/src/models.rs`, `crates/maidan-mcp/src/tools/{budget,approval,catalog}.rs` |
| **Argument strictness, all of them (398.6):** ranking by consequence is a correct reading of risk and the wrong basis for *which to fix* — a per-struct judgement has to be re-made every time someone adds a struct, and the person adding the 129th will not read the ranking. So every `Deserialize`-deriving `*Args` now rejects unknown fields, guarded by a static scan (`arg_strictness_contract.rs`) that also asserts it checked more than 100 structs, so a scan that stops matching reports itself instead of passing vacuously. Found along the way: **`build_mcp_arguments` was injecting four context ids into every MCP-tool slash dispatch**, including `list_channels`, which takes only `workspace_id` — strictness turned a silently-absorbed extra field into a `400`. Injection is now filtered by the tool's own `inputSchema`, and an unknown tool gets no injected context (fail closed). | `crates/maidan-mcp/src/tools/`, `crates/maidan-server/src/slash_commands.rs` |
| **A workspace handle is a display label, not an address (398.7):** decided rather than patched — the reverse lookup had no caller and inviting one would have made a mutable name into an identifier. ADR in [Decisions.md](Decisions.md). | `crates/maidan-store/` |
| **`Maidan-Room-LSN` reports the caller's room (398.8):** the header carried the *instance* head, so a caught-up projector could never reach the number it is compared against, and every tenant learned instance-wide event volume — shipped to third-party endpoints on outbound webhooks. `auth` now tags the response with a `RoomScope` extension from all four authenticating paths and the outer middleware reads it on the way back, which keeps one stamping site instead of three routers. An unscoped response falls back to the instance head, so the Cluster-390 "always on" contract holds. | `crates/maidan-server/src/{auth,room_lsn}.rs` |

**Claims corrected in the same sweep.** `docs/Claims.md` promises every claim maps
to a test or an honest "not yet". Two did not: *"every action is audited"* (28
audited action kinds against 150 mutating routes — the scope is by design, the
absolute was not), and *"cross-replica correctness"*, whose evidence cited the
transactional outbox — true, adjacent, and silent on whether the outbox is
relayed once, which it was not until 398.1.

## v397.0.0 — post-Cursor security audit remediation

Nine PRs (397.1–397.9). Clusters 377–396 were shipped autonomously by a Cursor
agent; a four-agent audit then found a systematic authorization gap. The run was
mechanically excellent — zero lint findings, zero TODOs, an e2e per feature — and
**every defect below passed CI and its own tests.**

One shape recurs: *something outranked the control meant to bind it.* Every test
asked "does the control work?"; none asked "what outranks it?"

| Change | Where |
|--------|-------|
| **Import scope (397.1):** `POST /workspaces/import` checked the per-workspace `token:admin` and never called `ensure_workspace` — the workspace written is the id inside a *caller-supplied bundle*, so tenant A could erase and replace tenant B, bypassing the Cluster-366 legal hold. A signature proves integrity, never authority. | `crates/maidan-server/src/routes/workspace.rs`, `crates/maidan-mcp/src/tools/export.rs` |
| **Gate ratchet (397.2):** both close-gates were guarded by `thread:transition` on *both* sides — the capability a close needs, and one `maidan.agent.worker` carries — so the constrained agent could delete its own constraint, untraceably. Loosening now needs `channel:admin` and is audited. **Four** operations loosen, not two: the fourth is `DELETE …/reviewers/:member_id`, because an empty named set means *any* non-implementer approval counts. | `crates/maidan-server/src/routes/{land_gate,review}.rs` |
| **Lag-resume watermark (397.3):** every bus consumer declared `watermark = 0` *inside* its consume loop, so a `Lagged` before the first event replayed the entire global log — for `fsm_hook_worker`, re-firing every historical hook through `dispatch_mcp_tool` with `AuthContext::bypass()`. Seeded from the log head at attach. Shipped inside #874. | `crates/maidan-server/src/{event_stream,webhook_worker,fsm_hook_worker,notification_router}.rs` |
| **Operator DLQ scope (397.4):** `GET /operator/egress/dead` + requeue ran global queries behind the per-workspace `token:admin` — read every tenant's Slack ids and repos, then requeue into them. | `crates/maidan-store/src/{postgres,sqlite}/egress_outbox.rs` |
| **Egress defusal (397.5):** four ways around Cluster 378.3's mention defusal — Slack escaping skipped code spans (`<!channel>` is Slack's own escape, not Markdown); the non-`reviewed` Slack body went out raw; one unmatched backtick disabled defusal for the rest of the body; a crafted `view_url` broke out of its Markdown link. | `crates/maidan-server/src/{egress_body,result_delivery}.rs` |
| **Federation wedge (397.6):** the origin chain covers every event, but federation accepts only the `federatable()` allowlist — and the link was recorded on the ingest path only, so one refused event wedged the peer permanently while the pull worker advanced past the loss. New `maidan_federated_verified_link`; the link is recorded *before* the policy check; the worker holds its cursor at the first failure. | `crates/maidan-server/src/{federation,federation_worker}.rs`, pg 0094 / sqlite 0093 |
| **Attenuation inheritance (397.7):** `attenuate` permits an equal capability list, so any bound the parent carried and the child did not could be shed by re-issuing. The derived token dropped `app_installation_id` (surviving the app being uninstalled) and per-token quotas. | `crates/maidan-server/src/routes/token.rs`, `crates/maidan-mcp/src/tools/room.rs` |
| **Chain integrity (397.8):** `backfill_chain` runs on every startup and its guard was global — one empty `content_hash` re-linked every row of every workspace against the *current* payloads, so edit-a-payload / blank-a-hash / restart made `verify_event_chain` report `ok: true`. A row that already carries a hash is now never rewritten; backfill is batched; `verify_chain` streams instead of collecting the whole log. | `crates/maidan-store/src/{postgres,sqlite}/events.rs`, `crates/maidan-types/src/event_chain.rs` |
| **Room-LSN DoS (397.9):** the header middleware sat *outside* the rate limiter, so a `429` still ran `MAX(id)` against the primary. The limiter is now outer and rejected responses skip the read. | `crates/maidan-server/src/{room_lsn,app}.rs` |

**Four items were written up as decisions rather than patched**, because each
needs a call rather than a diff: self-approval laundering (both gates test the
*live* `assignee_id`, so releasing a claim launders a self-approval — needs a
durable record of who did the work), `Maidan-Room-LSN` scoping (a published
contract across four SDKs), the search-indexer cursor (resuming trades away chain
re-verification — a correctness trade, not a perf one). **Handle resolution is
now decided** (Cluster 398.7): a handle is a display label, not an address — see
[Decisions.md](Decisions.md).

## v396.0.0 — Wave 3 #36 (partial): WASI slash-handler types

One PR (396.1). Landed the `SlashHandlerKind::wasi` variant, the invoke/result
lexicon types and handle validation. **Registrable on both write surfaces, and
every dispatch returns `wasi_runtime_unavailable`** — there is no runtime, no
feature flag, and it is documented nowhere else. Row #36 is **open**, not closed;
a user can successfully register a handler that can never run.

| Change | Where |
|--------|-------|
| **Types (396.1):** `WASI_INVOKE_TYPE` / `WASI_RESULT_TYPE`, handler-target validation (sha256), lexicon schemas. | `crates/maidan-types/src/{wasi,lexicon}.rs` |

## v395.0.0 — Wave 3 #35: named capability sets + stable `maidan://` URIs

Four impl PRs (395.1–395.4) + a retro. Named sets `maidan.agent.worker` / `maidan.human.admin` expand at mint time. Holders derive a weaker token without `token:admin` (Levy/Madden attenuation). Room URIs are `maidan://{workspace_id}/…` with an optional content-hash fragment; a handle rename cannot break stored ids. `GET /.well-known/maidan-room` is scheme-only. **Row #35 is closed.** Do **not** start #36 from this close.

| Change | Where |
|--------|-------|
| **Types (395.1):** `RoomUri` / `RoomCard` / `RoomDiscovery` / handle syntax. | `crates/maidan-types/src/{room_uri,room}.rs` |
| **Auth (395.2):** `named_sets` / `progressive_grant` / `attenuate` / `attenuate_expiry`. | `crates/maidan-auth/src/capability_set.rs` |
| **Store (395.3):** `maidan_workspace_handles` (pg 0093 / sqlite 0092). | `crates/maidan-store/src/{postgres,sqlite}/workspace_handles.rs` |
| **REST + MCP (395.4):** mint `capability_set`; `POST /tokens/attenuate`; well-known + room card + handle; MCP twins. | `crates/maidan-server/src/routes/{token,room}.rs`, `crates/maidan-mcp/src/tools/room.rs` |

## v394.0.0 — Wave 3 #34: tombstone explorer, backlink index, kind census

Three impl PRs (394.1–394.3) + a retro. Discover deleted messages honestly
(soft-delete + optional hard-purge reconstructions), query what points at
a message, and count `EventKind` in a workspace. No new table. **Row #34
is closed.** Do not start #35–36 from this close.

| Change | Where |
|--------|-------|
| **Types + store (394.1):** `TombstoneRecord` / `MessageBacklinks` / `KindCensus`; `IntegrityStore` on both backends. | `crates/maidan-types/src/explorer.rs`, `crates/maidan-store/src/{postgres,sqlite}/explorer.rs` |
| **REST (394.2):** `GET /workspaces/:id/tombstones`, `GET /messages/:id/backlinks`, `GET /workspaces/:id/kind-census`. | `crates/maidan-server/src/routes/{workspace,message}.rs` |
| **MCP (394.3):** `list_tombstones` / `list_message_backlinks` / `get_kind_census`. | `crates/maidan-mcp/src/tools/explorer.rs` |

## v393.0.0 — Wave 3 #33: snapshot catch-up + tap projector contract

Four impl PRs (393.1–393.4) + a retro. A peer that missed a pruned prefix takes a hashed `maidan.event-log.snapshot/1` checkpoint and catches up with `maidan.event-log.catch-up/1` pages (getRepo-shaped, not MST/CAR). Complements Cluster 392 (retained-suffix hash chain). Search is a tap projector and fails loud on a gap or chain break. **Row #33 is closed.** Do not start #34–36 from this close.

| Change | Where |
|--------|-------|
| **Types (393.1):** `LogSnapshot` / `CatchUpPage` / `verify_snapshot` / `verify_catch_up`; `TapContract` / `TapFault` / `SEARCH_PROJECTOR_KINDS`. | `crates/maidan-types/src/{log_snapshot,tap}.rs` |
| **Store (393.2):** `build_log_snapshot` + `catch_up_since`; workspace floor / head / at-or-before. | `crates/maidan-store/src/log_snapshot.rs` |
| **REST + MCP (393.3):** `GET /workspaces/:wid/snapshot`, `GET …/events/catch-up`; MCP `get_log_snapshot` / `catch_up_events` / `verify_event_chain`; CursorTooOld `snapshot` href. | `crates/maidan-server/src/routes/workspace.rs`, `crates/maidan-mcp/src/tools/event_log.rs` |
| **Search tap (393.4):** per-workspace verify on backfill; live waits for history; `Lagged` without a log → `RebuildRequired`. | `crates/maidan-search/src/{tap_projector,indexer}.rs` |

## v392.0.0 — Wave 3 #32: hash-chained log + strong refs

Four impl PRs (392.1–392.4) + a retro. Every stored event carries `{id, lsn, prev_hash, content_hash}` (SHA-256, `sha256:<hex>`). Peers detect a rewrite without trusting the host. `claim_next` and A2A citations pin `{uri, content_hash}`. Hashed, not signed; not MST/CAR; `lsn` is the event-log id, not WAL. **Row #32 is closed.** Do not start #33–36 from this close.

| Change | Where |
|--------|-------|
| **Types (392.1):** `$type` `maidan.event-log.chain/1`, `EventLink` / `StrongRef` / `verify_chain` / `verify_peer_link`. | `crates/maidan-types/src/event_chain.rs` |
| **Store (392.2):** columns on `maidan_events`; `append_in_tx` mints hashes; `Store::verify_event_chain`. | `crates/maidan-store/src/{postgres,sqlite}/events.rs`, migrations pg 0091 / sqlite 0090 |
| **REST + federation (392.3):** `GET /workspaces/:wid/events/verify` (200 / 409); origin-hash check on ingest. | `crates/maidan-server/src/routes/workspace.rs`, `federation.rs`; pg 0092 / sqlite 0091 |
| **Strong refs (392.4):** `ClaimedThread.pin` on `claim_next`; A2A `citations`. | `crates/maidan-server/src/routes/thread.rs`, `a2a_agent.rs`; `crates/maidan-a2a/src/protocol.rs` |

## v391.0.0 — Wave 3 #31: signed workspace export

Three impl PRs (391.1–391.3) + a retro. A workspace leaves the host as a signed `maidan.workspace.export/1` envelope a blank GHCR instance can verify without calling the origin. **Tokens die on export** — secrets are omitted; stuffed credential fields fail closed; mint new tokens after import. Operator Ed25519 key (`MAIDAN_EXPORT_SIGNING_KEY`); optional `MAIDAN_EXPORT_VERIFY_KEYS` authenticity pin. Not Room-LSN / not Consistency-Token. **Row #31 is closed.** Do not start #32–36 from this close.

| Change | Where |
|--------|-------|
| **Envelope (391.1):** `$type` `maidan.workspace.export/1`, `TokenPolicy::TokensDieOnExport`, canonical JSON + Ed25519. | `crates/maidan-types/src/signed_export.rs`, `crates/maidan-auth/src/export_sign.rs` |
| **REST (391.2):** signed `GET /workspaces/:id/export`, `POST /workspaces/export/verify`, signed `POST /workspaces/import`, `GET /operator/export-public-key`. All `token:admin`. | `crates/maidan-server/src/{export,routes/workspace}.rs` |
| **MCP (391.3):** `export_workspace` / `verify_workspace_export` / `import_workspace`; shared assemble/flatten. | `crates/maidan-mcp/src/tools/export.rs`, `crates/maidan-store/src/workspace_export.rs` |

## v390.0.0 — Wave 3 #30: EventKind lexicon, `$type`, Room-LSN

Four impl PRs (390.1–390.4) + a retro. An EventKind JSON-Schema pack (lexicon analogue) plus `$type` evolution (new fields optional, no renames, unknown ignored, breaking = new type). `Maidan-Room-LSN` is the event-log high-water so clients see projector / broadcast lag — **not** `Maidan-Consistency-Token` (WAL, replica-gated, Cluster 263). Canon snapshot tests over normalized wire shapes. SDK stays 0.1.0. **Row #30 is closed.**

| Change | Where |
|--------|-------|
| **Lexicon (390.1):** `EventKind::type_id` / pack under `contracts/lexicon/` + waiter / generic example kinds; `NEW-snapshot-tests`. | `crates/maidan-types/src/lexicon.rs`, `contracts/lexicon/` |
| **Room head (390.2):** `RoomLsn` + `Store::max_event_id` (both backends). Parse rejects WAL text. | `crates/maidan-types/src/room_lsn.rs`, `crates/maidan-store` |
| **Header (390.3):** always-on `Maidan-Room-LSN` on REST/WS/MCP/A2A; `$type` on live frames; `subscribe_ack.room_lsn`. | `crates/maidan-server/src/{room_lsn,event_stream,ws,mcp_stream}.rs` |
| **Projectors + SDK (390.4):** webhook `$type` + Room-LSN; four clients `last_room_lsn` (0.1.0). | `crates/maidan-server/src/webhooks.rs`, `sdk/` |

## v388.0.0 — Wave 3 #29: CursorTooOld, projector shapes, Lagged resume

Five impl PRs (388.1–388.5) + a retro. A subscribe / backfill cursor that points into a pruned gap **fails loud** (409 `must_refetch`) instead of silently clamping onto the remaining log. Projector shapes `{workspace, channel?, thread?, types[]}` filter HTTP backfill. Internal `BusItem::Lagged` consumers resume from the durable log. The SDK `follow` helper pages REST then cuts over to WS. Cluster 389 shipped first and left this number unused; 388 fills it. **Row #29 is closed.**

| Change | Where |
|--------|-------|
| **Foundation (388.1):** `CursorTooOld` / `ProjectorShape` / `ensure_cursor_fresh`. No new table. | `crates/maidan-types/src/cursor.rs`, `crates/maidan-store` |
| **Subscribe (388.2):** fail-loud on WS, MCP SSE, AG-UI, `GET …/events`; shape query params. | `crates/maidan-server/src/{delivery,ws,mcp_stream}.rs` |
| **Lagged (388.3):** `resume_from_log` + `maidan_bus_lag_resume_total` on webhook / notifications / FSM / indexer / AG-UI. | `crates/maidan-store/src/lag_resume.rs` |
| **SDK (388.4):** `follow` + `is_cursor_too_old` (Rust / Python / TS / Go). | `sdk/` |
| **E2e (388.5):** MCP SSE 409. | `crates/maidan-server/tests/cursor_too_old_subscribe_e2e.rs` |

## v389.0.0 — OSS hygiene: de-internalize / land-gate

One impl PR + a retro. The Cluster 385 close-gate keeps its semantics (pointer + pass/fail + green/amber/red; `closed` refuses without a qualifying green pass from a `land_gate`-skilled member ≠ owner/assignee) and is renamed to a public vocabulary any outsider can use. **`land_gate` / `LandGate` / `kind: "land_gate"`** everywhere (types, store table `maidan_thread_land_gate`, REST `/threads/:id/land-gate`, MCP `set/get/require/clear_land_gate`). Waiter examples are `example.review.result/1`; the frozen envelope is `maidan.waiter.result/1`; the delivery backlink is `view_url`. Internal product names (and internal repo selectors) are gone from the public surface. Raspberry Pi (`docs/Pi.md`) is unchanged. **386–387 were already used; 388 left unused.**

| Change | Where |
|--------|-------|
| **Rename (389.1):** close-gate public surface → land-gate across types, store, REST, MCP, OpenAPI, contracts, tests. Migrations rewritten in place (greenfield). | `crates/maidan-types/src/land_gate.rs`, `crates/maidan-store/src/{postgres,sqlite}/land_gate.rs`, `crates/maidan-server/src/routes/land_gate.rs`, `crates/maidan-mcp/src/tools/land_gate.rs` |
| **Scrub:** `example.*` result kinds, `maidan.waiter.result/1`, `view_url`, `example/repo` fixtures; docs/retros rewritten. | `crates/maidan-types/src/waiter.rs`, `docs/{Integration,Result Delivery,Architecture,Open Work}.md` |

## v387.0.0 — Wave 2 #28 (run-lineage half): the producer's `run_id` is the lineage

*Recorded late.* Three impl PRs (387.1–387.3, #818/#824/#828) plus an Open Work
note (#829) shipped with **no Capabilities entry, no CHANGELOG entry and no
Roadmap paragraph** — a table, five REST routes and four MCP tools with no record
anywhere. This section is that record. The cost of the gap was not abstract:
**C5** — `run_occupancy` never learning about `maidan_thread_blocks` — sat
undetected in the one surface nobody had written down, and was fixed in Cluster
400.1.

| Change | Where |
|--------|-------|
| **Lineage foundation (387.1):** `maidan_thread_lineage` (pg 0090 / sqlite 0089) homes a producer's `run_id` on a thread as `parent_run_id`. **The design call: accept the producer's id, do not mint a parallel one.** A second identifier would have to be correlated back to the first by every consumer, and the producer already has one that means something to it. Migration numbers were picked to clear in-flight 385 (land gate) and 386 (blocked reasons). | `crates/maidan-store/src/{postgres,sqlite}/thread_lineage.rs` |
| **Nested occupancy (387.1):** `run_occupancy(workspace, run_id)` partitions every open thread sharing the value into `queued` / `claimed` / `working` / `blocked` over the Cluster-351 two clocks. **F7 mute stays orthogonal** — a muted nested thread still counts, because muting is about who gets told, not about whether the work exists. | `thread_lineage.rs` |
| **REST (387.2):** `PUT` / `GET` / `DELETE /threads/:id/lineage`, `GET /workspaces/:id/run-threads`, `GET /workspaces/:id/run-occupancy`. `set_thread_result` **auto-homes** the run id when the waiter envelope carries one, so the common path needs no explicit call. | `crates/maidan-server/src/routes/{thread,workspace}.rs` |
| **MCP (387.3):** `set_thread_lineage` / `get_thread_lineage` / `list_run_threads` / `get_run_occupancy`, with the same auto-home from `set_thread_result`. **Delete stays REST-only** — clearing lineage is an operator correction, not agent work. | `crates/maidan-mcp/src/tools/thread.rs` |

**Wave 2 #28 is not closed by this.** Follow-a-member occupancy and the manager
digest are the remaining halves.

## v386.0.0 — Wave 2 #27: a closed blocked-reason enum

Four impl PRs (386.1–386.4) + a retro. An orchestrator parks a thread from dispatch with a **closed** `BlockedReason` (`dag|gate|human|child|quota|unclaimable`) — unlike `result_kind`, a namespaced string. `claim_next` skips a `maidan_thread_blocks` row. Clearing emits `BlockedResolved` (non-federatable). Distinct from Cluster 218 DAG-children-must-be-terminal and Cluster 363's unclaimable park table (`unclaimable` here is vocabulary, not a replacement). **Row #27 is closed.**

| Change | Where |
|--------|-------|
| **Store (386.1):** `BlockedReason` + `ThreadBlock`; `maidan_thread_blocks` (pg 0089 / sqlite 0088); set/clear/get/list. Zero blast on `claim_next`. | `crates/maidan-types/src/models.rs`, `crates/maidan-store/src/{postgres,sqlite}/blocks.rs` |
| **Skip (386.2):** all four `claim_next` SQL sites + queue-depth `ready`/`blocked` + occupancy. 218 DAG clause stays. | `crates/maidan-store/src/{postgres,sqlite}/threads.rs` |
| **Event (386.3):** `BlockedResolved` + `clear_thread_block_with_event` (one tx). | `crates/maidan-types/src/events.rs`, `contracts/event-kinds.json` |
| **REST/MCP/e2e (386.4):** `PUT`/`GET`/`DELETE /threads/:id/block` + `GET /channels/:cid/blocked`; MCP twins; explicit claim 409; bus observe. | `crates/maidan-server/src/routes/{thread,channel}.rs`, `crates/maidan-mcp/src/tools/thread.rs` |

## v385.0.0 — Wave 2 #25 remainder: LandGate gate pointer + green/amber/red

Four impl PRs (385.1–385.4) + a retro. A thread holds `{kind:"land_gate", status:pass|fail, artifact_sha?, land}`. Presence of a row arms the close-gate (no row = vacuous green, Cluster 375 shape). `closed` refuses unless a **green pass** from a `land_gate`-skilled member ≠ owner/assignee. Amber (flags-then-still-engages) is not a land. Fail is always red. Room holds the pointer; an external verifier records pass/fail. Not a CI product / a judge panel. Cluster 384 is P1.1d (closed by this retro). **Row #25 is closed** (383 composition + 385 pointer).

| Change | Where |
|--------|-------|
| **Types + store (385.1):** `LandGatePointer` / `LandColor` / standing; table pg 0088 / sqlite 0087; require / set / get / clear. Unskilled writes `InvalidInput`. | `crates/maidan-types/src/land_gate.rs`, `crates/maidan-store/src/{postgres,sqlite}/land_gate.rs` |
| **FSM (385.2):** `transition_in_tx` refuses `closed` unless a qualifying green pass (or no row). | `crates/maidan-store/src/{postgres,sqlite}/thread_transitions.rs` |
| **REST + MCP (385.3):** `PUT`/`GET`/`DELETE /threads/:id/land-gate` + `PUT …/requirement`; tools `set/get/require/clear_land_gate`. | `crates/maidan-server/src/routes/land_gate.rs`, `crates/maidan-mcp/src/tools/land_gate.rs` |
| **e2e (385.4):** HTTP close-gate + MCP standing; fail stays red. | `crates/maidan-server/tests/land_gate_e2e.rs` |

## v384.0.0 — P1.1d: MCP `transition_thread` twin of the REST FSM

One impl PR (384.1) + a retro. MCP `transition_thread` advances a thread's FSM (`start_review` / `close` / `archive`) through `transition_thread_with_event` + `publish_stored`. SoD, the required-reviewers close-gate, unresolved `refutes`, and the Cluster-383 critical composition apply identically — no MCP bypass. Terminal transitions emit `ThreadReady` for newly-ready dependents. **P1.1d is closed.** Cluster 385 (LandGate) is independently on `main`. Wave 2 #26–28 / Wave 3/4 are not this work.

| Change | Where |
|--------|-------|
| **MCP `transition_thread` (384.1):** `{thread_id, actor_id, action}` → `transition_thread_with_event` + `publish_stored`; `ThreadReady` on terminal; resource URIs; 5-place wiring + both sorted contracts. `maidan-fsm` is a runtime dep. | `crates/maidan-mcp/src/tools/{thread,mod,catalog}.rs`, `resource_updates.rs`, `contracts/mcp-*.json` |
| **Tests:** happy path + SoD denial + close-gate refusal; Cluster 383 critical-result e2e now closes via the tool. | `crates/maidan-mcp/src/server.rs` |

## v383.0.0 — Wave 2 #25 composition: critical waiter findings → Cluster-375 `request_changes`

Three impl PRs (383.1–383.3) + a retro. A reviewed `example.review.result/1` whose `findings` contain any `critical` is a `request_changes` from a review-skilled agent. If the thread has no requirement, the adapter arms Cluster-375 `k=1` so `closed` refuses until a third-party human approves. Owner/assignee approvals still do not count (SoD). No new gate machinery. GitHub review `event` stays `COMMENT` (380). **The #25 composition is closed.** The LandGate pointer + green/amber/red vocabulary shipped as Cluster 385.

| Change | Where |
|--------|-------|
| **Types + store (383.1):** `review_decision_from_waiter` + `apply_critical_review_decision` (skill-gated upsert). Severity walked on the raw findings array. | `crates/maidan-types/src/{waiter,review}.rs`, `crates/maidan-store/src/{postgres,sqlite}/reviews.rs` |
| **Arm k + bus (383.2):** `set_requirement(1)` when unset; `ThreadResultSet` → `arm_critical_review`. Empty `deliver_to` still arms. | `crates/maidan-server/src/{result_delivery,notification_router}.rs` |
| **Write-path + e2e (383.3):** REST `PUT /threads/:id/result` + MCP `set_thread_result` arm immediately; HTTP/MCP e2e prove close 409 until a human approve. | `crates/maidan-server/src/routes/thread.rs`, `crates/maidan-mcp/src/tools/thread.rs`, `crates/maidan-server/tests/critical_review_e2e.rs` |

## v381.0.0 — Wave 2 #24 facet half: `result_kind` is a namespaced-string list

Four impl PRs (381.1–381.4) + a retro. 381.4 documented the facet; it is not the retro. Thread results are listed by the **namespaced string** a producer publishes (`example.review.result/1`), not a closed `decision|plan|merge_authorized` enum and not the ADR convention `"kind": "decision"`. The surface is a workspace-scoped list (`GET /workspaces/:id/results` + MCP `list_thread_results`), exact-match, not message-FTS. Omit `result_kind` to list every accessible non-tombstoned result; private-channel rows the caller cannot read are dropped. Cluster 382's `list_channel_closed_results` is untouched. **Row #24 is closed** (382 pack + 381 facet). **The result-delivery arc (377–381) is COMPLETE.** Clusters 380 and 382 stay closed. This close does not start Wave 2 #25.

| Change | Where |
|--------|-------|
| **Store (381.1):** `result_kind_from_payload` (string alone; missing/empty/non-string → `None`; no waiter-schema requirement) + indexed column on `maidan_thread_results` (pg 0087 / sqlite 0086) + `list_thread_results(workspace, kind, limit)` both backends. Re-set updates or clears the facet. | `crates/maidan-types/src/models.rs`, `crates/maidan-store/src/{postgres,sqlite}/thread_results.rs` |
| **REST (381.2):** `GET /workspaces/:id/results?result_kind=&limit=` (`workspace:read`). Full new-route preflight. `can_access_thread` post-filter. | `crates/maidan-server/src/routes/workspace.rs` |
| **MCP (381.3):** `list_thread_results` twin; workspace from the token; 5-place wiring + both sorted contracts. | `crates/maidan-mcp/src/tools/{thread,mod,catalog}.rs` |
| **Docs (381.4):** Discoverability is the namespaced string; `"kind": "decision"` is not `?result_kind=decision`. | `docs/Result Delivery.md`, `docs/Integration.md` |

## v380.0.0 — inline per-finding PR review comments

Three impl PRs (380.1–380.3) + a retro. After a successful Cluster 379 GitHub summary comment, a `reviewed` envelope with `head_sha` and usable findings posts `POST /repos/{repo}/pulls/{n}/reviews` with `commit_id = head_sha` (never the live PR head), `event: COMMENT`, GitHub **RIGHT**, `line` = `line_range.end`. Missing sha / empty findings / non-`reviewed` / Slack skip the review without sinking the summary. 404/422 meter `skipped`; 5xx/auth meter `failed` (replay retries the review). Review errors never `disable_link`. Cluster 379's summary path is unchanged. **Cluster 381 is not unparked** (already open: the `result_kind` facet).

| Change | Where |
|--------|-------|
| **Frame (380.1):** `line_range` is 1-indexed inclusive **post-image** lines at `head_sha`; GitHub RIGHT; `github_line()` = `end`; `github_start_line()` only when `start != end`. `review_commit_id()` is envelope `head_sha` only. | `crates/maidan-types/src/waiter.rs`, fixture `waiter_result_v1.json` |
| **Review POST (380.2):** `GithubSender::create_review` after the 379 summary; `event: COMMENT`; cap 100 comments; mention-defused finding bodies; no 379 marker on inline comments. Metric `maidan_github_review_total{outcome}`. | `crates/maidan-server/src/{github.rs,egress_worker.rs,result_delivery.rs}` |
| **Skip vs fail (380.3):** 404/422 → `{skipped}`; 5xx / rate-limited 403 / 401/403 → `{failed}` + replay; never `disable_link`; dual-surface review only on GitHub; vanished envelope skips the review; projector rows never `create_review`. | `crates/maidan-server/tests/result_delivery_inline_e2e.rs`, `egress_wire_e2e.rs` |

## v382.0.0 — Wave 2 #24 pack half: claimer pack includes accepted decisions

Three impl PRs (382.1–382.3) + a retro. The next `claim_next` claimer sees
in-channel **accepted/closed decisions** as token-lean teasers on the live
thread pack (`GET /threads/:id/context` + MCP `get_thread_context`).
`claim_next` itself still returns `Option<Thread>`. Waiter envelopes
(`schema = maidan.waiter.result/1`) appear only when `status` is `reviewed`;
`result_kind` is a **namespaced string** (e.g. `example.review.result/1`), not a
closed enum. Full payloads stay on `GET /threads/:id/result`. **The other
half of row #24** (facet `result_kind` into search) remains **Cluster 381**.

| Change | Where |
|--------|-------|
| **Store (382.1):** `list_channel_closed_results` — JOIN results × threads, `closed`/`archived`, non-tombstoned, newest `produced_at` first; `exclude_thread_id`; limit clamped `1..=50`. Dumb: no JSON interpretation. | `crates/maidan-store/src/{postgres,sqlite}/thread_results.rs`, `crates/maidan-types/src/models.rs` |
| **REST pack (382.2):** `AcceptedDecision` teasers on live `ThreadContext` (default on, cap 10, opt-out `include_accepted_decisions=false`). Waiter envelopes only when `reviewed`; opaque JSON on a terminal thread is accepted; withheld on DM / as-of / workspace-nested. | `crates/maidan-types/src/pack.rs`, `crates/maidan-server/src/thread_context.rs` |
| **MCP twin (382.3):** same field on `get_thread_context` / `snapshot_thread_context` (MCP has its own assembler). Catalog arg default true. | `crates/maidan-mcp/src/context.rs`, `crates/maidan-mcp/src/tools/catalog.rs` |

## v379.0.0 — the result-delivery primitive

Five impl PRs (379.1–379.5) + a retro. Clusters 377 and 378 made projector egress durable, aimable, and repeatable; this is the producer's actual ask. A `maidan.waiter.result/1` envelope written with `set_thread_result` is now fetched, parsed, allowlist-checked per `deliver_to` target, and delivered — GitHub gets `rendered`, Slack gets `summary`, a re-review updates the same object, and the producer reads per-target disposition over REST + MCP. Empty `deliver_to` ⇒ nowhere (valid). Non-`reviewed` ⇒ a Maidan-authored failure notice from `status` alone. **The grammar is frozen** at `maidan.waiter.result/1`. Cluster 380 (inline per-finding comments) is **unparked as next**: `head_sha` is on the fixture; 380.1 still pins the `line_range` frame of reference.

| Change | Where |
|--------|-------|
| **Store (379.1):** `maidan_result_deliveries` (pg 0085 / sqlite 0084, `UNIQUE (thread_id, surface, selector)`) + `ResultDelivery` / `arm_result_delivery` (wins iff `revision > armed_revision`, keeps `external_ref`) / `mark_result_delivered` / `mark_result_delivery_failed` / skip. **Two watermarks:** `armed_revision` (seen — the monotonic dedup) and `delivered_revision` (landed). Arming against only `delivered_revision` cannot tell a second replica of the *same* revision (both read `NULL`, one must lose) from a newer result arriving in-flight (must win). Intent/identity, not transport — the outbox stays Cluster 377. | `crates/maidan-types/src/result_delivery.rs`, `crates/maidan-store/src/{postgres,sqlite}/result_deliveries.rs` |
| **Contract lock (379.2):** `parse_waiter_result`, a pure tolerant reader with `DeliverTarget::Unknown(String)`. Fixture `crates/maidan-types/tests/fixtures/waiter_result_v1.json` — a producer-side grammar change breaks this test. Unrecognized `schema` ⇒ no delivery. | `crates/maidan-types/src/waiter.rs` |
| **Trigger (379.3):** `ThreadResultSet` arm in `notification_router::route_event` → `result_delivery.rs` (fetch → parse → per-target allowlist check then enqueue). Skip is recorded (`status=skipped`), not an error. `maidan_result_deliveries_total{outcome}`. | `crates/maidan-server/src/{result_delivery.rs,notification_router.rs}` |
| **Update-in-place (379.4):** stored `external_ref` → `update_*`; GitHub recovery marker `<!-- maidan:result:<thread_id> -->` at byte 0 (reserved inside the 65536-char ceiling). `EgressKind {Projector, Result}` on the outbox (pg 0086 / sqlite 0085) so a projector post to the same issue cannot PATCH the result comment. Result 401/403/404 dead-letters **without** `disable_link`. | `crates/maidan-server/src/{egress_worker.rs,egress_body.rs}`, `crates/maidan-types/src/egress.rs` |
| **Status + replay (379.5):** `GET /threads/:id/deliveries` (`workspace:read`) + `POST …/deliveries/:did/replay` (`workspace:write`) + MCP `list_result_deliveries` / `replay_result_delivery`. Replay reopens as `pending` without bumping `armed_revision`, enqueues with a synthetic negative `source_log_id`, re-checks the allowlist (unblessed stays skipped). Audit `result_delivery.attempt` / `result_delivery.replay`. | `crates/maidan-server/src/routes/thread.rs`, `crates/maidan-mcp/src/tools/delivery.rs`, `crates/maidan-store/src/result_delivery.rs` |

## v378.0.0 — the egress trust boundary + the sender upgrade

Three impl PRs (378.1–378.3) + a retro. Cluster 377 made projector egress durable; this makes it **safe to aim** and **safe to repeat**. A destination must be blessed by an operator before Maidan will post to it (**`deliver_to` selects, the allowlist authorizes** — default empty ⇒ deliver nowhere); a sender now says what it created and can edit it later, which is what makes a re-review an update rather than a second comment; and a body arriving on either surface is mention-free, within the surface's ceiling, and in that surface's own markup. **Nothing delivers a result yet** — the last foundation before Cluster 379 wires the primitive.

| Change | Where |
|--------|-------|
| **Allowlist (378.1):** `maidan_egress_targets` (pg 0084 / sqlite 0083, `UNIQUE (workspace_id, surface, selector)`) + `AllowedEgressTarget`/`NewEgressTarget` + `allow_egress_target` (idempotent) / `list_egress_targets` / `revoke_egress_target` (workspace-scoped) / `is_egress_target_allowed`, both backends. A result's `deliver_to` is agent-written while the connector credentials are operator-held — routing straight off it would make Maidan a confused deputy. | `crates/maidan-types/src/egress.rs`, `crates/maidan-store/src/{postgres,sqlite}/egress_targets.rs` |
| **Allowlist REST (378.1):** `POST`/`GET /workspaces/:wid/egress-targets` + `DELETE …/:tid`, all **`token:admin`** — reads included, because the allowlist is *policy* and letting a workspace-scoped token enumerate it would hand an agent the list of destinations worth aiming at. Both mutations audited. A selector must be an **id** (Slack `C…`/`G…`, GitHub `owner/name`), never a mutable name; on GitHub the blessing is the **repository**, so one blessing covers every PR in it. | `crates/maidan-server/src/routes/egress_targets.rs` |
| **Sender upgrade (378.2):** `ExternalRef` (`Slack { channel_id, ts }` / `Github { repo, comment_id }`) returned from both `post_*`, plus `SlackSender::update_message` (`chat.update`) and `GithubSender::update_comment` (`PATCH /repos/{repo}/issues/comments/{id}`); `post_message` gained `thread_ts` so a re-delivery replies in-thread. **A post that succeeded is never reported as a failure** — `Ok(None)` means "posted, but not addressable", because an `Err` would make the at-least-once worker retry and leave two comments. | `crates/maidan-server/src/{slack.rs,github.rs,egress_worker.rs}` |
| **Body projection (378.3):** `egress_body.rs`, pure — a GitHub mention defused by wrapping it in a **code span** (a documented rendering rule, and visible, rather than a zero-width space), Slack's `<!…>`/`<@…>` escaped to `&lt;`, truncation to GitHub's 65536-character ceiling that says so and keeps the backlink, and a **deliberately narrow** GFM→mrkdwn projection. A shared code segmenter keeps all three rules out of fenced diffs. | `crates/maidan-server/src/egress_body.rs` |

## v377.0.0 — durable projector egress (row #38; the result-delivery foundation)

Four impl PRs (377.1–377.4) + a retro. The Slack and GitHub projectors posted inline and best-effort: a transient 502 dropped the message with a log line and nothing else. A projector-bound message is now **enqueued** and delivered by a retry/backoff worker, an auth/config-class failure **disables the link** and announces it rather than retrying forever, and a delivery that exhausts its retries lands in a `token:admin` DLQ an operator can inspect and replay. **Nothing is silently dropped** — the hard prerequisite for the result-delivery arc (378–381), since "recorded and auditable per target" is not implementable on log-and-drop. **A queue, not a new connector:** what the projectors say and where they say it is unchanged.

| Change | Where |
|--------|-------|
| **Store (377.1):** `maidan_egress_outbox` (pg 0082 / sqlite 0081, modelled on `maidan_mail_outbox`) + `EgressSurface`/`EgressTarget`/`EgressOutbox` + `EgressStore` — `enqueue` (`ON CONFLICT DO NOTHING`) / `claim_next_due` (pg `FOR UPDATE SKIP LOCKED`, sqlite serialized CAS) / `mark_delivered` / `mark_failed` / `count_dead`, both backends. `UNIQUE (source_log_id, surface, selector)` is load-bearing (every replica enqueues — the Cluster-238 lesson); `source_log_id` carries **no FK** so event-log retention cannot cascade into a queued delivery. | `crates/maidan-types/src/egress.rs`, `crates/maidan-store/src/{postgres,sqlite}/egress_outbox.rs` |
| **Worker (377.2):** `egress_worker.rs` (the `mail_worker.rs` sibling) — lease 120 s, backoff 30 s→1 h, dead-letter at 8, 1000/tick, `MAIDAN_EGRESS_WORKER_TICK_SECS`. `route_message_to_{slack,github}` enqueue instead of posting; the link lookup + loop-prevention check stay at enqueue. Spawned only when a projector sender is configured, and `sweep_once` is a no-op without one. | `crates/maidan-server/src/{egress_worker.rs,slack.rs,github.rs,main.rs}` |
| **Retry-then-disable (377.3):** `disabled_at` on both link tables (pg 0083 / sqlite 0082, `NULL` = enabled) + `disable_{slack_channel,github_issue}_link`; an auth/config-class failure disables the link and the enqueue then skips it. Per-surface `is_misconfiguration` allowlists (GitHub 401/403/404; Slack's error *string*) — and **a rate-limited 403 is explicitly not one**, read from `x-ratelimit-remaining: 0` / `retry-after`. Ingress is untouched. Re-linking clears `disabled_at`. | `crates/maidan-server/src/{slack.rs,github.rs,egress_worker.rs}`, `crates/maidan-store/src/{postgres,sqlite}/{slack,github}_links.rs` |
| **`ProjectorMisconfigured` (377.3):** a broken connector credential is a room event (`{workspace, channel?, thread, surface, selector, error}`, non-federatable — a peer has no standing to declare our credentials broken); `error` is the surface's own words. Plus `maidan_egress_deliveries_total{surface,outcome}` (`sent`/`retry`/`dead`/`disabled`/`unroutable`), the queue-level companion to the unchanged per-surface post counters. | `crates/maidan-types/src/events.rs`, `crates/maidan-server/src/metrics.rs`, `contracts/event-kinds.json` |
| **Operator DLQ (377.4):** `GET /operator/egress/dead` + `POST /operator/egress/dead/{id}/requeue`, both `token:admin` (the Cluster-306 mail-DLQ shape — the queue is cross-workspace) over `DeadEgress` + `list_dead_egress`/`requeue_dead_egress`. A row answers *what failed, where was it going, what did the surface say* without a log dive. | `crates/maidan-server/src/routes/egress_ops.rs`, `crates/maidan-store/src/{postgres,sqlite}/egress_outbox.rs` |

## v376.0.0 — Wave 2 #23: a spawn budget (G6 + G-dev-3 + W3)

Six impl PRs (376.1–376.6) + a retro. A workspace caps how far an agent family may fan out — `max_children` per parent claim, `max_depth` nesting, `max_tools` per thread — and a spawn past the cap is refused (409 `SpawnRejected`) instead of admitting one more agent onto a late claim; coordination cost grows as n(n−1)/2. Each axis is opt-in (`null` = unlimited). A claim's **external** fan-out is capped too: at most one GitHub issue/PR link. **A budget, not a scheduler.**

| Change | Where |
|--------|-------|
| **Store (376.1):** `maidan_spawn_budgets` (pg 0080 / sqlite 0079; `workspace_id` PK, three nullable limits) + `SpawnBudget` + `SpawnBudgetStore` — set (whole-row upsert; all-`None` clears) / get, plus the gate's counts `count_active_children` / `thread_depth` (ancestor CTE, root = 1) / `count_thread_tool_uses` (Cluster-173 `tool_use` blocks), both backends. | `crates/maidan-types/src/spawn.rs`, `crates/maidan-store/src/{postgres,sqlite}/spawn.rs` |
| **Children + depth gate (376.2):** `enforce_spawn_budget` after `validate_parent` in **both** thread-create paths, both backends — so REST, MCP, recipes and the scheduler inherit it. Root threads + no-budget workspaces unrestricted. | `crates/maidan-store/src/{postgres,sqlite}/threads.rs` |
| **Max-tools gate (376.3):** `enforce_tool_budget` in both post paths — a cumulative per-thread cap, run only when the post carries tool-use blocks, so an ordinary message stays off the extra-query path. | `crates/maidan-store/src/{postgres,sqlite}/messages.rs` |
| **Config (376.4):** REST `PUT`/`GET /workspaces/:id/spawn-budget` (`workspace:write`/`workspace:read`) + MCP `set_spawn_budget`/`get_spawn_budget`. `PUT` is a full replace (`{}` clears, `0` freezes an axis); `GET` is total (all axes `null` when unset, no 404). | `crates/maidan-server/src/routes/workspace.rs`, `crates/maidan-mcp/src/tools/spawn.rs` |
| **GitHub-link cap (376.5):** the reverse index on `maidan_github_issue_links(thread_id)` becomes UNIQUE (pg 0081 / sqlite 0080) — with the existing `(repo, issue_number)` key that is a bijection: one thread per issue **and** one issue per thread. The store maps the violation to a `Conflict`; no route/tool change. | `migrations/{postgres/0081,sqlite/0080}_github_link_cap.sql`, `crates/maidan-store/src/{postgres,sqlite}/github_links.rs` |
| **`ThreadSpawnDenied` (376.6):** a refusal is a room event (`{workspace, channel, thread, member, axis, limit, observed}`, non-federatable) from the REST thread-create route, both REST post branches, and the MCP post tool; the gate returns a typed `StoreError::SpawnRejected(SpawnDenial)` so the payload isn't parsed out of a message. | `crates/maidan-types/src/{events.rs,spawn.rs}`, `crates/maidan-server/src/routes/mod.rs`, `crates/maidan-mcp/src/tools/message.rs` |
| **Integrator docs (376.7):** a "Spawning helpers has a ceiling" section — the three axes, read/set, the non-retryable `409` / `-32602`, that the caps are *lifetime* budgets (a closed child keeps its slot; only a tombstone frees it) not concurrency limits, and the one-GitHub-link-per-claim rule. | `docs/Integration.md` |

## v375.0.0 — Wave 2 #22: required reviewers (G5 + G-dev-5)

Four impl PRs (375.1–375.4) + a retro (+ a CI chore, #760). A thread's `closed` transition is gated on `k` distinct **qualifying** approvals (reviewer ≠ owner/assignee — separation of duties — and, when a named set exists, in it) AND no unresolved `refutes` edge. **A gate, not a poll/closer.** Maintainer's design call: a dedicated review store.

| Change | Where |
|--------|-------|
| **Store (375.1):** `maidan_thread_review_reqs` (k) + `maidan_thread_reviewers` (named n) + `maidan_thread_reviews` (decisions) (pg 0079 / sqlite 0078) + `ReviewDecision`/`ThreadReview`/`ThreadReviewRequirement`/`ReviewStatus` + `ReviewStore` (`review_status` = distinct qualifying-approval count), both backends. | `crates/maidan-types/src/review.rs`, `crates/maidan-store/src/{postgres,sqlite}/reviews.rs` |
| **Close-gate (375.2):** `review_gate_in_tx` in the FSM `transition_in_tx` (both backends), on `to_state == Closed`: refuses close unless approvals met + no `refutes` edge targets the thread → `Conflict`. Additive. | `crates/maidan-store/src/{postgres,sqlite}/thread_transitions.rs` |
| **REST (375.3):** `review-requirement` PUT/GET/DELETE, `reviewers` POST/GET + DELETE `:member_id`, `reviews` POST/GET, `review-status` GET (`thread:transition` writes, `workspace:read` reads). | `crates/maidan-server/src/routes/review.rs` |
| **MCP (375.4):** `set_review_requirement`/`add_reviewer`/`submit_review` + `get_review_status`/`list_reviews`. | `crates/maidan-mcp/src/tools/review.rs` |

## v374.0.0 — P1.1c: the MCP assignment dual-write (the P0)

One impl PR (374.1) + a retro. Closes the last MCP write-path-parity gap the transactional-outbox migration (205–214) was meant to cover, surfaced by a 2026-09-10 audit. The MCP assignment tools now match REST's crash-consistency — and a reclaim finally emits `ClaimExpired` on the agent-primary surface. No new Wave number (folds under the outbox program).

| Change | Where |
|--------|-------|
| **Atomic MCP assignment (374.1):** `assign`/`claim`/`unassign`/`claim_next`/`release_claim` use their `*_with_event` store variants + `publish_stored`; the `publish_assignment` helper is deleted. `claim_next` publishes every returned event → a reclaim emits `ClaimExpired` (dead holder) + `ThreadAssignmentChanged`. | `crates/maidan-mcp/src/tools/thread.rs` |
| **Conflict mapping (374.1):** `StoreError::Conflict` → `McpError::InvalidParams` (a client error, not `-32603` Internal). | `crates/maidan-mcp/src/error.rs` |

## v373.0.0 — Wave 2 #21: attachable labeled memory as room objects (H11)

Four impl PRs (373.1–373.4) + a retro. A **memory block** is a Letta-shaped `{label, description, limit, read_only, value}` workspace object attachable to a thread. A parent watches a child's result block **without a nested runtime** via the reactive `MemoryBlockUpdated` event + the MCP `wait_for_memory_block` long-poll. Full rewrite, last-writer-wins — **not a transcript, not RAG**.

| Change | Where |
|--------|-------|
| **Store (373.1):** `maidan_memory_blocks` (pg 0078 / sqlite 0077, `UNIQUE(workspace, label)`) + `maidan_thread_memory_blocks` + `MemoryBlock`/`MemoryBlockId` + pure `fits_char_limit`/`is_valid_block_label` + `MemoryBlockStore` (concurrent-safe create, full-rewrite `set_value`, attach/detach), both backends. | `crates/maidan-types/src/memory_block.rs`, `crates/maidan-store/src/{postgres,sqlite}/memory_blocks.rs` |
| **REST (373.2):** CRUD under `/workspaces/:wid/memory-blocks[/:id]` + attach/detach/list under `/threads/:id/memory-blocks[/:block_id]`, `workspace:read`/`write`; read-only/over-limit → 400, cross-tenant → 404. | `crates/maidan-server/src/routes/memory_block.rs` |
| **MCP (373.3):** label-addressed `create`/`get`/`list`/`set`/`attach`/`detach`/`list_thread` memory-block tools. | `crates/maidan-mcp/src/tools/memory_block.rs` |
| **Reactive watch (373.4):** `MemoryBlockUpdated` event on set-value (REST + MCP, non-federatable "go fetch" pointer) + MCP `wait_for_memory_block` long-poll. | `crates/maidan-types/src/events.rs`, `crates/maidan-mcp/src/tools/memory_block.rs` |

## v372.0.0 — Wave 2 #20: a freeze-member kill-switch (G17 + B25)

Four impl PRs (372.1–372.4) + operator docs. An operator (or an orchestrator agent) freezes a compromised/runaway **member**: it drops their leases, `claim_next` refuses them, and they stay frozen until an explicit unfreeze. **Not G4 PAUSE** (which pauses a thread/workspace) — this stops one member.

| Change | Where |
|--------|-------|
| **Store (372.1):** `maidan_member_freezes` (pg 0077 / sqlite 0076) + `MemberFreezeStore` — `freeze_member` records the freeze + drops the member's active leases in one tx (returns the count released); unfreeze/is-frozen/get/list, both backends. | `crates/maidan-types/src/freeze.rs`, `crates/maidan-store/src/{postgres,sqlite}/member_freezes.rs` |
| **Claim enforcement (372.2):** both `claim_next` variants refuse a frozen member via an atomic `NOT EXISTS` clause. | `crates/maidan-store/src/{postgres,sqlite}/threads.rs` |
| **REST (372.3):** `POST/DELETE/GET /members/:id/freeze` + `GET /workspaces/:wid/frozen-members`, `token:admin`, audited. | `crates/maidan-server/src/routes/freeze.rs` |
| **MCP (372.4):** `freeze_member`/`unfreeze_member`/`list_frozen_members` — the first `token:admin` MCP tools. | `crates/maidan-mcp/src/tools/freeze.rs` |
| **Operator docs:** a "Kill switches" catalog (the freeze API + the `MAIDAN_*` env flags). | `docs/Operations.md` |

## v371.0.0 — Wave 2 #19: secret-ref (G19 + T3)

Four impl PRs (371.1–371.4). A named secret whose **value never enters the event log** — the log carries a `secret://<name>` reference, the store holds the AEAD-encrypted value, and it's materialized only transiently: a consumer resolves it at exec, or the egress broker substitutes it on the way out to an allowlisted host.

| Change | Where |
|--------|-------|
| **Store (371.1):** `maidan_secrets` (pg 0076 / sqlite 0075, ciphertext only) + `Secret`/`SecretId` + pure `secret://` ref helpers (`secret_refs_in`, `substitute_secret_refs`) + `SecretStore` CRUD (create = rotate), both backends. | `crates/maidan-types/src/secret.rs`, `crates/maidan-store/src/{postgres,sqlite}/secrets.rs` |
| **REST + caps (371.2):** `secret:read`/`secret:admin`; create (encrypts) / list (metadata) / `resolve` (decrypts — "a consumer fetches at exec") / delete; the value crosses the wire only on create + resolve. | `crates/maidan-server/src/routes/secret.rs` |
| **MCP (371.3):** `list_secrets` / `resolve_secret`; `McpServer` gains an at-rest key set at startup. | `crates/maidan-mcp/src/tools/secret.rs` |
| **Egress broker (371.4):** substitutes `secret://` refs on a webhook delivery only for hosts on `MAIDAN_SECRET_EGRESS_ALLOWLIST`; a non-allowlisted host gets the literal ref; substitution at send time, never persisted. | `crates/maidan-server/src/secret_broker.rs`, `webhook_worker.rs` |

## v370.0.0 — Wave 2 #18: a recipe / thread-type (G8 + W5 + G-dev-9)

Five impl PRs (370.1–370.5). A **recipe** is a reusable thread-type blueprint (the Goose-recipe *shape*: params, a definition of done, a retry policy, inline child sub-tasks forming a DAG). Instantiating one builds a parent thread + its DAG children + attaches skills, freezing the recipe bytes into a run snapshot (copy-on-fire). A schedule can seed a run, skipping when the prior run is still in flight. **Not a recipe VM** — a blueprint the room instantiates.

| Change | Where |
|--------|-------|
| **Store foundation (370.1):** `maidan_recipes` (pg 0073/sqlite 0072) + `RecipeSpec` types + pure `validate` (acyclic child DAG via Kahn) + `validate_params` + `RecipeStore` CRUD, both backends. | `crates/maidan-types/src/recipe.rs`, `crates/maidan-store/src/{postgres,sqlite}/recipes.rs` |
| **Instantiation (370.2):** `maidan_recipe_runs` (pg 0074/sqlite 0073) + `instantiate_recipe` — parent + DAG children + skills + copy-on-fire snapshot, in one tx (reuses `create_thread_with_event` + the dep/skill tables). | `recipes.rs`, `store.rs` |
| **REST (370.3):** create/list/get/delete + `instantiate` (→ RecipeRun, publishes ThreadCreated); full new-route preflight. | `crates/maidan-server/src/routes/recipe.rs` |
| **MCP (370.4):** `create_recipe` / `list_recipes` / `instantiate_recipe`. | `crates/maidan-mcp/src/tools/recipe.rs` |
| **Scheduled runs (370.5):** `task_schedules.recipe_id` (pg 0075/sqlite 0074); the sweeper instantiates the recipe per firing, or emits `ScheduleSkipped` (new non-federatable EventKind) when the prior run is still in flight. | `crates/maidan-server/src/scheduler.rs`, `crates/maidan-types/src/events.rs` |

## v369.0.0 — Wave 2 #17: an AG-UI door (H1)

Two impl PRs (369.1–369.2): a second front-end protocol on the event stream — [AG-UI](https://docs.ag-ui.com), what CopilotKit and agent IDEs speak. A **thread is a run**, so the door is a *view* over the existing resumable bus, not a new runtime. Output direction only (Maidan → AG-UI); the UI→agent input direction is a follow-up.

| Change | Where |
|--------|-------|
| **Event types + pure mapping (369.1):** `AgUiEvent` (AG-UI wire shape) + `agui_events_for(&Event)` — `ThreadCreated`→`RUN_STARTED`, terminal `ThreadStateChanged`→`RUN_FINISHED`/non-terminal→`STEP_STARTED`, `ClaimFailed`→`RUN_ERROR`, `MessagePosted`→`TEXT_MESSAGE_*` + `TOOL_CALL_*` per content block, `ThreadLanded`→`CUSTOM`. Pure + unit-tested. | `crates/maidan-server/src/agui.rs` |
| **SSE door (369.2):** `GET /agui/stream` (workspace/channel/thread scoped) emits the mapped frames; reuses `/mcp/stream` bus-subscribe + replay; `Last-Event-ID`/`after_id` resume with per-frame source `id:`; per-event RBAC (`can_access_thread`/`can_access_channel`); off-contract (`event:subscribe` inline, like `/mcp/stream`). | `crates/maidan-server/src/agui_stream.rs`, `app.rs` |

## v368.0.0 — Wave 2 #16: the waiting-on-you inbox

A stacked cluster (368.1–368.3, G15/G9): a member-centric aggregate of everything needing their attention — assigned tasks, open gates, unread mentions — one member's queue, each aged against an SLA. Not `@everyone`.

| Change | Where |
|--------|-------|
| **Aggregate + REST (368.1):** pure `assemble_waiting_inbox` (WaitingKind/Item/Inbox) — drops terminal/tombstoned assigned threads, merges 3 sources, sorts oldest-first, flags overdue; `GET /members/:id/waiting?sla_secs=N` composes 3 existing store reads + the assembler (no new store code). | `crates/maidan-types/src/models.rs`, `crates/maidan-server/src/routes/member.rs` |
| **MCP (368.2):** `get_waiting_inbox` over the shared assembler. | `crates/maidan-mcp/src/tools/member.rs` |
| **`/ui` (368.3):** a "Waiting on you" section atop the Work tab, oldest first, overdue flag, tunable SLA. | `crates/maidan-server/static/index.html` |

## v367.0.0 — Wave 2 #15: the human work console (`/ui`)

A stacked `/ui` cluster (367.1–367.3) letting a human inhabit the workplace loop — vanilla, no SPA. All the machinery shipped in Wave 1; this surfaces it.

| Change | Where |
|--------|-------|
| **Work tab (367.1, B2):** channel queue depth (224) + occupancy (351), threads, a thread's result (234) + DAG deps (217), task schedules (226). 5 `/ui/api` reads. | `crates/maidan-server/static/index.html`, `crates/maidan-server/src/app.rs` |
| **Prefs console (367.2, B11):** delivery mode (256), email (250), muted kinds (242), channel + thread follows (245) — self-only. 12 `/ui/api` routes. | `crates/maidan-server/static/index.html`, `crates/maidan-server/src/app.rs` |
| **Looking glass (367.3, B3):** events by kind, thread by id, artifact by sha (404 → not-found), peers — read-only explorer. 1 new `/ui/api` read. | `crates/maidan-server/static/index.html`, `crates/maidan-server/src/app.rs` |
| Guards: `ui_js_wires_{work,prefs,looking_glass}_tab` static checks + `work.spec.ts` / `glass.spec.ts` Playwright specs. | `crates/maidan-server/tests/ui_js_contract.rs`, `ui-tests/tests/` |

## v366.0.0 — Wave 1 #14: legal hold, OTel gate, web push, SCIM

Four independent tracks (the backlog's "four bullets, not one cluster"), each shipped as its own PR to `main`.

| Change | Where |
|--------|-------|
| **T6 legal hold (366.1):** `maidan_legal_holds` (pg 0070 / sqlite 0069). A held workspace's events survive retention pruning (in-SQL `NOT IN`), audit pruning freezes, purge/erase → 409. REST place/lift/get (`token:admin`) + `/operator/legal-holds`. | `crates/maidan-store/src/*/legal_hold.rs`, `crates/maidan-server/src/routes/workspace.rs` |
| **H15 OTel feature-gate (366.2):** OTLP trace + metrics is a default-on cargo feature `otel`; `--no-default-features` compiles the OpenTelemetry/tonic stack out (plain tracing + Prometheus scrape stay); the bootstrap-strip job covers the no-otel build. | `crates/maidan-observability/{Cargo.toml,src/*}`, `crates/maidan-server/{Cargo.toml,src/metrics.rs}` |
| **N1 web push (366.3):** `maidan_push_subscriptions` (pg 0071 / sqlite 0070) + VAPID (RFC 8292) + aes128gcm encryption (RFC 8291), RustCrypto (no openssl). Router delivers iff no live WS; `410 Gone` prunes. REST register/list/delete. | `crates/maidan-server/src/web_push.rs`, `crates/maidan-store/src/*/push_subscriptions.rs` |
| **SCIM-as-OIDC-P3 (366.4):** `/scim/v2/` (ServiceProviderConfig + Users create/read/list-filter/replace/patch/delete); `maidan_scim_users` (pg 0072 / sqlite 0071); deactivation/delete revoke tokens; `token:admin`, outside OpenAPI+map (the `/mcp` precedent). | `crates/maidan-server/src/scim.rs`, `crates/maidan-store/src/*/scim_users.rs` |

## v365.0.0 — fair dispatch (Wave 1 #13 cont.)

A stacked cluster (365.1–365.4, G3) giving a thread a **dispatch priority** with **aging**, so `claim_next` is no longer strict FIFO. A high-priority task jumps the queue, but a long-waiting normal task ages one rank per hour until it overtakes newer higher-priority work — priority alone would starve the low end; the aging makes it *fair*. With this, **Wave 1 #13 is complete** (WIP 362 + Unclaimable 363 + wait-edges 364 + fair dispatch 365).

| Change | Where |
|--------|-------|
| **Store (365.1):** `maidan_thread_priorities` (pg 0069 / sqlite 0068); `ThreadPriority`; `set_thread_priority`/`get_thread_priority`; absence = default 0. | `migrations/*/006{9,8}_thread_priorities.sql`, `crates/maidan-store/src/{sqlite,postgres}/priorities.rs` |
| **Dispatch (365.2):** both `claim_next` variants order by `priority + floor(age_seconds / 3600)` DESC (created_at tiebreak); pg `FOR UPDATE OF c`, sqlite `strftime('%s',…)`. The by-id `claim` is untouched. | `crates/maidan-store/src/{sqlite,postgres}/threads.rs` |
| **REST (365.3):** `PUT`/`GET /threads/:id/priority`. | `crates/maidan-server/src/routes/thread.rs` |
| **MCP (365.4):** `set_priority`/`get_priority`. | `crates/maidan-mcp/src/tools/thread.rs` |

## v364.0.0 — wait-edges + on_timeout escalation (Wave 1 #13 cont.)

A stacked cluster (364.1–364.5, G2/G4) giving a thread a **durable wait timer** with an escalation policy. A thread declares a deadline and an `on_timeout` action; either it is cancelled (the awaited thing happened) or a background sweeper fires it on timeout — reaching the thread's owner and optionally **parking** the thread (reuse of Cluster 363). It steals the Restate/Temporal timer *shape* — it is not a workflow engine, and `on_timeout` **never invents a decision** (reach + park only, the "TimedOut ≠ Decline" rule). With this, **Wave 1 #13 is complete** (WIP 362 + Unclaimable 363 + wait-edges 364).

| Change | Where |
|--------|-------|
| **Store (364.1):** `maidan_thread_waits` (pg 0068 / sqlite 0067); `EscalationPolicy` (`Notify`/`Park`); `set_thread_wait`/`cancel_thread_wait`/`get_thread_wait` + `claim_next_due_wait` (atomic fire-once). | `migrations/*/006{8,7}_thread_waits.sql`, `crates/maidan-store/src/{sqlite,postgres}/waits.rs`, `crates/maidan-types/src/models.rs` |
| **Event (364.2):** `WaitTimedOut` (non-federatable), naming the `policy` applied. | `crates/maidan-types/src/events.rs` |
| **Sweeper + escalation (364.3):** opt-in `wait_sweeper.rs` fires due waits (`Park` → unclaimable) + publishes `WaitTimedOut`; the notification router notifies the thread owner. `maidan_wait_timed_out_total{policy}`. | `crates/maidan-server/src/{wait_sweeper,notification_router,main}.rs` |
| **REST (364.4):** `PUT`/`DELETE`/`GET /threads/:id/wait`. | `crates/maidan-server/src/routes/thread.rs` |
| **MCP (364.5):** `set_wait`/`cancel_wait`/`get_wait`. | `crates/maidan-mcp/src/tools/thread.rs` |

## v363.0.0 — Unclaimable (Wave 1 #13 cont.)

A stacked cluster (363.1–363.4, G3) letting a thread be **parked from dispatch** with a reason — distinct from blocked-by-deps, blocked-by-gate, and skill-miss. A parked thread stays open but `claim_next` skips it and an explicit `claim` is refused (409), until un-parked.

| Change | Where |
|--------|-------|
| **Store (363.1):** `maidan_thread_unclaimable` (pg 0067 / sqlite 0066); `mark_thread_unclaimable`/`mark_thread_claimable`/`get_thread_unclaimable`/`list_unclaimable_threads`; presence = parked. | `migrations/*/006{7,6}_thread_unclaimable.sql`, `crates/maidan-store/src/{sqlite,postgres}/unclaimable.rs` |
| **Dispatch (363.2):** `claim_next` skips parked threads (both backends); `QueueDepth` gains an `unclaimable` bucket (4-way partition of `open`). | `crates/maidan-store/src/{sqlite,postgres}/threads.rs`, `crates/maidan-types/src/models.rs` |
| **REST (363.3):** `PUT`/`DELETE /threads/:id/unclaimable` + `GET /channels/:cid/unclaimable`; explicit `claim` → 409 on a parked thread. | `crates/maidan-server/src/routes/{thread,channel}.rs` |
| **MCP (363.4):** `mark_unclaimable`/`mark_claimable`/`list_unclaimable` + the claim refusal. | `crates/maidan-mcp/src/tools/thread.rs` |

## v362.0.0 — the WIP limit (Wave 1 #13)

A stacked cluster (362.1–362.3, G11) giving a workspace a **work-in-progress cap**: the max concurrent **live** claims any one member may hold. An agent can no longer grab unbounded concurrent work; a capped member's `claim_next` finds nothing and an explicit `claim` is refused (409). Counts live claims, never queued-never-started ghosts.

| Change | Where |
|--------|-------|
| **Store foundation (362.1):** `maidan_wip_limits` (pg 0066 / sqlite 0065) — per-workspace cap; `set_wip_limit`/`get_wip_limit` + `count_live_claims` (complement of the `claim_next` predicate). No row = unlimited; `0` = frozen. | `migrations/*/006{6,5}_wip_limits.sql`, `crates/maidan-store/src/{sqlite,postgres}/wip.rs`, `crates/maidan-store/src/store.rs` |
| **REST enforcement + admin (362.2):** `claim_next` → null / `claim` → 409 at the cap (`routes::at_wip_limit`); `PUT`/`GET /workspaces/:wid/wip-limit` + `GET /members/:id/wip`. | `crates/maidan-server/src/routes/{mod,thread,workspace}.rs` |
| **MCP enforcement + tools (362.3):** same on `claim_thread`/`claim_next_thread`; `set_wip_limit`/`get_wip_limit`/`get_member_wip` tools. | `crates/maidan-mcp/src/tools/thread.rs`, `crates/maidan-mcp/src/tools/{mod,catalog}.rs` |

## v361.0.0 — the landed fact (Wave 1 #12)

A stacked cluster (361.1–361.4, G-dev-7) that **steals the landed fact**: an inbound `pull_request.merged` webhook on a linked PR becomes a durable `ThreadLanded` event, which reaches the accountable owner + followers and can be awaited over MCP. Not an automation product — the fact is recorded; the thread's FSM is not touched.

| Change | Where |
|--------|-------|
| **`ThreadLanded` event (361.1):** `EventKind`/`Event::ThreadLanded {repo, pr_number, merged_by?, merge_commit_sha?, title?}`; non-federatable; full EventKind drill + contracts. | `crates/maidan-types/src/events.rs`, `contracts/event-kinds.json`, `crates/maidan-server/src/federation.rs` |
| **Projector ingress (361.2):** `POST /integrations/github/events` `pull_request` merge → `ThreadLanded` on the linked thread (reuses `get_github_issue_link`); does not transition the FSM. | `crates/maidan-server/src/github.rs` |
| **Notification reach (361.3):** the router notifies the thread's owner + followers on land (mute-honoring). | `crates/maidan-server/src/notification_router.rs` |
| **`wait_for_landed` MCP (361.4):** block until a thread's PR lands (`thread_id`/`channel_id`-scoped, `since_log_id` lookback, RBAC-filtered) — the `wait_for_ready` analogue. | `crates/maidan-mcp/src/tools/thread.rs`, `crates/maidan-mcp/src/tools/{mod,catalog}.rs` |

## v360.0.0 — the token-budgeted context pack (Wave 1 #11)

A stacked cluster (360.1–360.4, G-dev-1) making the scoped context pack budget itself by **tokens**, not just rows: give it a `token_budget` and it keeps the thread's framing (opening message) and its recent tail, folds the elided middle into an auditable `elision` marker ("Lost in the Middle"), and — for a child task — grounds the pack in its parent's ask and decision.

| Change | Where |
|--------|-------|
| **Pack primitive (360.1):** `maidan_types::pack` — `estimate_tokens` (`chars/4`), `message_tokens`, `PackElision`, `fold_messages_to_budget` (keep opener + recent tail, fold the middle). Pure, unit-tested. | `crates/maidan-types/src/pack.rs`, `crates/maidan-server/tests/token_pack.rs` |
| **Token-budgeted REST pack (360.2):** `?token_budget=N` on `GET /threads/:id/context` (+ workspace pack, per nested thread) → folds before the refs/edits/artifacts reads; `ThreadContext.elision`. | `crates/maidan-server/src/thread_context.rs`, `crates/maidan-server/src/{dto,routes/thread,routes/workspace,openapi/mod}.rs` |
| **Token-budgeted MCP pack (360.3):** `token_budget` on `get_thread_context`/`snapshot_thread_context`/`get_workspace_context`; `elision` on the response + catalog schemas. | `crates/maidan-mcp/src/context.rs`, `crates/maidan-mcp/src/tools/catalog.rs` |
| **Child grounds (360.4):** `ParentGrounding` (parent's opening ask + latest decision) on a child thread's pack; `include_parent_grounding` (default true). Withheld for cross-channel / DM / tombstoned parents. | `crates/maidan-types/src/pack.rs`, `crates/maidan-server/src/thread_context.rs`, `crates/maidan-mcp/src/context.rs` |

## v359.0.0 — inbox & search depth (Wave 1 #10)

A stacked cluster (359.1–359.4, N2 / N5 / N4) making the notification inbox legible — group by thread, snooze the noise, surface the decisions you missed — and search time-scopable.

| Change | Where |
|--------|-------|
| **Date-range search (359.1, N4):** `SearchFilters {after, before}` — a half-open window on `posted_at`, both backends × lexical + semantic (hybrid inherits); REST + MCP. | `crates/maidan-search/src/{filters,postgres,sqlite}.rs`, `crates/maidan-server/src/routes/search.rs`, `crates/maidan-mcp/src/tools/search.rs` |
| **Notification snooze (359.2, N5):** `snoozed_until` (pg 0065 / sqlite 0064) — snoozed notifications leave the inbox + badge, resurface on lapse; `POST /members/:id/notifications/:nid/snooze` + MCP. | `migrations/*/00{65,64}_notification_snooze.sql`, `crates/maidan-store/src/*/notifications.rs`, `crates/maidan-server/src/routes/member.rs` |
| **Inbox grouped by thread (359.3, N5):** `group_notifications_by_thread` (pure) → one group/thread; `GET /members/:id/notifications/grouped` + MCP. | `crates/maidan-types/src/models.rs`, `crates/maidan-server/src/routes/member.rs`, `crates/maidan-mcp/src/tools/member.rs` |
| **Buried-decisions digest (359.4, N2):** the digest leads with `ThreadResult`s a member missed in followed channels/threads; `Store::buried_decisions_for_member` + `GET /members/:id/decisions` + MCP. | `crates/maidan-store/src/*/email_digest.rs`, `crates/maidan-server/src/digest.rs` |

## v358.0.0 — the budget envelope (Wave 1 #9)

A stacked cluster (358.1–358.4, T1/T5) giving a task/run a **budget envelope** — token, USD, turn, and wall-clock maxima — that **stops the run** when exceeded, records the stop as a **failure** (not a close), and **dead-letters** it for triage. Over REST + MCP.

| Change | Where |
|--------|-------|
| **Budget + usage store (358.1):** `maidan_thread_budgets` (pg 0063 / sqlite 0062) — optional `max_{tokens,usd_micros,turns,wall_secs}` + accumulated `used_*`; `BudgetStore` set/get/add-usage; `ThreadBudget::exceeded`. USD as integer micros; wall from the Cluster-351 working clock. | `migrations/*/00{63,62}_thread_budgets.sql`, `crates/maidan-store/src/*/budget.rs`, `crates/maidan-types/src/models.rs` |
| **`ClaimFailed` + agent-work DLQ (358.2):** `EventKind::ClaimFailed` (hard stop ≠ success; non-federatable) + `maidan_agent_work_dlq` (pg 0064 / sqlite 0063) + record/list. | `crates/maidan-types/src/events.rs`, `migrations/*/00{64,63}_agent_work_dlq.sql`, `crates/maidan-store/src/*/dlq.rs` |
| **Enforcement + REST (358.3):** `report_thread_usage` — over-budget on a claimed thread atomically releases the claim + `ClaimFailed` + DLQ; `PUT`/`GET /threads/:id/budget`, `POST /threads/:id/usage`, `GET /channels/:cid/dlq`. | `crates/maidan-store/src/*/budget.rs`, `crates/maidan-server/src/routes/{thread,channel}.rs` |
| **MCP (358.4):** `set_thread_budget` / `get_thread_budget` / `report_usage` / `list_dlq`. | `crates/maidan-mcp/src/tools/budget.rs` |

Budget-exhaustion is a claim-level failure, not a new terminal thread state; enforcement is at the report heartbeat (no reaper). Not a billing SKU.

## v357.0.0 — scoped notification mute (Wave 1 #8)

A stacked cluster (357.1–357.3, N3) adding a **per-channel** mute with **mention breakthrough** — silence a busy channel's firehose while still getting @mentioned. With Cluster 356.3's per-thread mute, notification mute is now scopeable at kind, channel, and thread granularity.

| Change | Where |
|--------|-------|
| **Per-channel mute (357.1):** `maidan_channel_mutes` (pg 0062 / sqlite 0061) + `mute_channel`/`unmute_channel`/`is_channel_muted`/`channel_muters` (both backends). | `migrations/*/00{62,61}_channel_mutes.sql`, `crates/maidan-store/src/*/follows.rs` |
| **Router + mention breakthrough (357.2):** the router drops a channel-muter from the `MessagePosted` fan-out + `notify` path, but a `MentionRecorded` pierces a channel mute. Hierarchy: kind-mute > thread-mute (suppresses even a mention) > channel-mute (pierced by a mention). Plus `POST`/`DELETE /channels/:cid/mute`. | `crates/maidan-server/src/notification_router.rs`, `crates/maidan-server/src/routes/channel.rs` |
| **MCP tools (357.3):** `mute_channel` / `unmute_channel`. | `crates/maidan-mcp/src/tools/channel.rs` |

**Deferred (N3 sub-item):** the *projector-kind overlay* (muting by Slack/GitHub projector origin) — a separate design.

## v356.0.0 — the threading cluster (Wave 1 #7)

A stacked cluster (356.1–356.5, F1 + F2 + F7) making a thread a first-class, titled, navigable object: a parent's replies collapse to per-child summaries, a post floats its thread up an activity-ordered list, a thread can be renamed after creation, and a member can mute one thread without leaving the channel.

| Change | Where |
|--------|-------|
| **Collapsed child threads (356.1):** `ChildThreadSummary` + `Store::child_thread_summaries` + `GET /threads/:id/children` — "N replies" per child without loading its messages. | `crates/maidan-types/src/models.rs`, `crates/maidan-store/src/*/threads.rs`, `crates/maidan-server/src/routes/thread.rs` |
| **Thread activity bump (356.2):** a post bumps its thread's `updated_at` in-tx + `Store::list_recently_active_threads` + `GET /channels/:cid/recent-threads`. No new event (`MessagePosted` carries the `thread_id`). | `crates/maidan-store/src/*/{messages,threads}.rs`, `crates/maidan-server/src/routes/thread.rs` |
| **Leaf mute (356.3):** `maidan_thread_mutes` (pg 0061 / sqlite 0060) + store mute/unmute/is-muted/muters + `POST`/`DELETE /threads/:id/mute`; the notification router skips a muted recipient (per-kind-independent). | `migrations/*/00{61,60}_thread_mutes.sql`, `crates/maidan-store/src/*/follows.rs`, `crates/maidan-server/src/notification_router.rs` |
| **Rename thread (356.4):** `Store::set_thread_title` + `PUT /threads/:id/title` + MCP `rename_thread` (blank → 400). A rename does not bump the activity clock. | `crates/maidan-store/src/*/threads.rs`, `crates/maidan-server/src/routes/thread.rs`, `crates/maidan-mcp/src/tools/thread.rs` |
| **Threading MCP parity (356.5):** `list_child_threads`, `list_recently_active_threads`, `mute_thread`/`unmute_thread`. | `crates/maidan-mcp/src/tools/thread.rs` |

**Deferred (stretch sub-item of #7):** *Automerge as thread collab* (causal edits on the same titled thread) — the event log stays the log; a separate, larger design.

## v355.0.0 — the owner/steer cluster (Wave 1 #6)

A stacked cluster (355.1–355.5, W1) giving a task thread a durable **owner** (the accountable party, distinct from the assignee/claimer), enforcing that the claimer cannot land its own owned work, persisting steering guidance across handoffs, and notifying the owner when an owned task gets stuck.

| Change | Where |
|--------|-------|
| **Owner axis (355.1):** `Thread.owner_id` (pg 0059 / sqlite 0058) + `Store::set_thread_owner`, orthogonal to the claim axis. | `migrations/*/00{59,58}_threads_owner.sql`, `crates/maidan-store/src/*/threads.rs`, `crates/maidan-types/src/models.rs` |
| **Owner REST + separation of duties (355.2):** `PUT`/`DELETE /threads/:id/owner`; a terminal transition by the assignee on an owner-governed thread is rejected in the shared transition core. | `crates/maidan-server/src/routes/thread.rs`, `crates/maidan-store/src/*/thread_transitions.rs` |
| **Persist steer (355.3):** `maidan_thread_steer` + `ThreadSteerStore` + `PUT`/`GET /threads/:id/steer` (latest wins, survives handoffs). | `migrations/*/00{60,59}_thread_steer.sql`, `crates/maidan-store/src/*/thread_steer.rs` |
| **Notify owner on stuck (355.4):** the router turns a `ClaimExpired` on an owned thread into a notification to the owner. | `crates/maidan-server/src/notification_router.rs` |
| **MCP surface (355.5):** `set_thread_owner` / `set_thread_steer` / `get_thread_steer`. | `crates/maidan-mcp/src/tools/thread.rs` |

Owner-governance is opt-in (setting an owner enables SoD); un-owned threads are unrestricted.

## v354.0.0 — the wait contract (Wave 1 #5)

A stacked cluster (354.1–354.3, H4) hardening the `wait_for_*` long-polls. The waits were live-only — a signal that fired between the caller's last drain and the subscribe was silently missed (the drain/subscribe race). They now take an opt-in `since_log_id` lookback that closes the gap, and the surrounding contract (resume, idempotency, evict-on-wait) is written down.

| Change | Where |
|--------|-------|
| **Member-wait lookback (354.1):** `wait_for_mention` / `wait_for_notification` gain `since_log_id`; replay the log for a matching event before parking live. | `crates/maidan-mcp/src/tools/member.rs`, `catalog.rs` |
| **Thread/workspace-wait lookback (354.2):** `wait_for_result` / `wait_for_ready` / `wait_for_claim_expired` the same, via a shared `lookback_event`. | `crates/maidan-mcp/src/tools/thread.rs`, `catalog.rs` |
| **The wait contract (354.3):** the no-occupancy-I/O-in-Drop invariant codified at `PresenceRegistration::drop`; the resume / idempotency / evict-on-wait contract documented in Integration.md. | `crates/maidan-server/src/presence.rs`, `docs/Integration.md` |

Gapless by construction (subscribe-before-lookback), RBAC-preserving, opt-in (omit `since_log_id` for pure-live). No new store surface.

## v353.0.0 — the identity chrome (Wave 1 #4)

A stacked cluster (353.1–353.4) giving the vanilla `/ui` the human-facing chrome for the 350/351/352 mechanics — what a session can do, where each task's occupant sits, that a token cannot widen its grant — all keyboard-operable to WCAG 2.1 AA. No SPA; every badge and card derives from data the backend already serves (the only backend touch was one additive `WhoAmI` field + one proxy route).

| Change | Where |
|--------|-------|
| **Capability card (353.1):** a Session tab renders `{can, can't}` from `/me`'s real grant; `WhoAmI.known_capabilities` + `/ui/api/me`. | `crates/maidan-server/src/{dto,app,routes/member}.rs`, `crates/maidan-server/static/index.html` |
| **Session-chrome badges (353.2):** per-thread running / idle / needs-input / needs-approval / done, from assignee + working clock + state + gate schema. | `crates/maidan-server/static/index.html` |
| **Attenuation chrome (353.3):** minting cannot widen the caller's grant (`capsExceedingGrant`, the client mirror of `validate_subset`). | `crates/maidan-server/static/index.html` |
| **WCAG-AA tablist + skip link (353.4):** ARIA roles, roving tabindex, Arrow/Home/End, skip link, `:focus-visible`. | `crates/maidan-server/static/index.html` |

Guarded by four new `ui_js_contract` static checks + four Playwright specs (`session`, `session-chrome`, `attenuation`, `a11y`) + a `ui_session_e2e` Rust test.

## v352.0.0 — the HITL list (Wave 1 #3)

A stacked cluster (352.1–352.4) bringing A2A `tasks/list` up to the live A2A 1.0.0 conformance bar, so an **external** A2A agent can discover a pending Cluster-350 held gate by polling `tasks/list?status=input-required` — the gate it must answer now shows up as an `input-required` task.

| Change | Where |
|--------|-------|
| **Gates as `input-required` tasks (352.1):** `gate_as_task` synthesizes a task view from a pending approval gate at read time; `tasks/get` falls back to a gate lookup, `tasks/list` leads with gate-tasks. RBAC-checked; no real task materialized. | `crates/maidan-server/src/a2a_agent.rs`, `crates/maidan-a2a/src/protocol.rs` |
| **`status` filter + `pageSize` max 100 (352.2):** `normalize_task_state` (kebab/enum/bare → canonical); `pageSize` clamps `1..=100`. | `crates/maidan-a2a/src/protocol.rs`, `crates/maidan-server/src/a2a_agent.rs` |
| **`application/a2a+json` + `includeArtifacts` (352.3):** REST §11 media type; `includeArtifacts` accepted on get + list (omitted when false). | `crates/maidan-server/src/a2a_agent.rs`, `crates/maidan-a2a/src/protocol.rs` |
| **`statusTimestampAfter` filter (352.4):** `list_a2a_tasks` gains `updated_after` (both backends); malformed → 400. | `crates/maidan-store/src/{store,postgres/a2a,sqlite/a2a}.rs`, `crates/maidan-server/src/a2a_agent.rs` |

Deferred: 352.5 real `nextPageToken` keyset paging (Open Work — needs RBAC-in-query first; the ≤`pageSize` case is already conformant).

## v351.0.0 — the occupancy clocks (Wave 1 #2)

A multi-PR cluster (351.1–351.6): a live picture of where every task-thread's work sits, plus fencing against zombie holders. The centrepiece is **two clocks** — the claim clock (lease) and the working clock (acknowledge) — which separate a claimed-but-idle agent from one actively working.

| Change | Where |
|--------|-------|
| **Claim fencing (351.1–351.2):** a `claim_lease_id` token (pg 0057 / sqlite 0056) minted on every claim/assign, cleared on unassign; `renew`/`acknowledge`/`release` fenced on `(assignee_id, claim_lease_id)` so a reclaimed-out stale holder is rejected. | `migrations/*/005{6,7}_*`, `crates/maidan-store/src/*/threads.rs`, `crates/maidan-types/src/{ids,models}.rs` |
| **The working clock (351.3):** `work_started_at` (pg 0058 / sqlite 0057) reset on every (re)claim, stamped by `acknowledge_claim` (idempotent). REST `POST /threads/:id/claim/acknowledge` + MCP `acknowledge_claim`. | `crates/maidan-store/src/*/threads.rs`, `crates/maidan-server/src/routes/thread.rs`, `crates/maidan-mcp/src/tools/thread.rs` |
| **The occupancy view (351.4):** `GET /channels/:cid/occupancy` + MCP `get_channel_occupancy` → `{open, queued, claimed, working, blocked}` (the two-clocks refinement of `QueueDepth`). | `crates/maidan-store/src/*/threads.rs`, `crates/maidan-server/src/routes/channel.rs`, `crates/maidan-mcp/src/tools/thread.rs` |
| **`release_claim` (351.5):** REST `POST /threads/:id/claim/release` + MCP `release_claim` — a fenced graceful handoff returning work to the queue immediately. | `crates/maidan-store/src/*/threads.rs`, `crates/maidan-server/src/routes/thread.rs`, `crates/maidan-mcp/src/tools/thread.rs` |
| **`ClaimExpired` event + `wait_for_claim_expired` (351.6):** a distinct filterable "an agent died" signal, emitted lazily+atomically when `claim_next` reclaims an expired lease (`member_id` = dead holder); non-federatable. MCP long-poll consumer. | `crates/maidan-types/src/events.rs`, `crates/maidan-store/src/*/threads.rs`, `crates/maidan-mcp/src/tools/thread.rs`, `contracts/event-kinds.json` |

The claim lifecycle is now **claim → acknowledge → renew → release**, every step fenced. The remaining occupancy thickener (G1's formal two-clocks invariant model) is deferred to Program V.

## v350.0.0 — the held gate (durable human approval) (Wave 1 #1)

A multi-PR cluster (350.1–350.8) replacing the blocking MCP elicitation model with a durable, queryable approval gate: an agent asks for approval, gets an async *input-required* handle, and a human answers later over REST or the `/ui` — the answer integrity-checked.

| Change | Where |
|--------|-------|
| **Durable gate (350.1–350.3):** `maidan_approval_gates` (pg 0056 / sqlite 0055) + compare-and-set `resolve`; MCP `request_approval` returns an async `input_required` handle (no held socket) + `get_approval_gate`; REST `GET /workspaces/:wid/approval-gates` + `POST /approval-gates/:id/answer` with an HMAC-signed `requestState` (409 double-answer / 400 bad action / 403 bad transition). | `migrations/*/0055/0056_approval_gates`, `crates/maidan-store/src/*/approval_gates.rs`, `crates/maidan-mcp/src/tools/approval.rs`, `crates/maidan-server/src/routes/approval_gate.rs` |
| **Required-human claim gate (N6, 350.6):** a pending gate on a thread blocks `claim_next` — an agent is never handed work waiting on a person. | `crates/maidan-store/src/*/threads.rs` |
| **Retire sampling+roots + the dead `request_client` subsystem (350.4–350.5):** removed the deprecated `summarize_thread`/`list_roots` tools and the now-unreachable server→client machinery; kept `2024-11-05` + `Last-Event-ID` resumability. | `crates/maidan-mcp/src/{server.rs,streamable_session.rs,tools/}` |
| **A Playwright `/ui` test harness (350.7):** a seed-and-serve Rust example + a `ui-tests/` headless-Chromium project driving the real `/ui` in CI — the first browser test in the repo. | `crates/maidan-server/examples/ui_test_server.rs`, `ui-tests/`, `.github/workflows/ci.yml` |
| **An Approvals tab in the `/ui` (350.8):** lists + answers pending gates (accept/decline/cancel) — the human elicitation client. | `crates/maidan-server/static/index.html`, `crates/maidan-server/src/app.rs` |

## v349.0.0 — deferred-work wrap-up (audit close-out)

A multi-PR cluster (349.1–349.5) closing every remaining deferred item from the post-flagship audit.

| Change | Where |
|--------|-------|
| **Store trait split (349.1):** the 258-method `Store` god-trait → 35 cohesive domain sub-traits (`WorkspaceStore`/`ThreadStore`/`MessageStore`/…) + a method-less `Store` super-trait + blanket impl, so `dyn Store` call sites are unchanged and a caller can bound on a narrow sub-trait; `maidan_store::prelude` re-exports them for concrete-backend callers. | `crates/maidan-store/src/{store.rs,lib.rs,sqlite/mod.rs,postgres/mod.rs}` |
| **Notification batch INSERT (349.2):** `Store::create_notifications_batch` writes the unmuted follower set in one `INSERT … ON CONFLICT DO NOTHING RETURNING` (PG `UNNEST` / SQLite chunked `VALUES`) → the fan-out is ~2 round trips regardless of follower count. | `crates/maidan-store/src/{store.rs,*/notifications.rs}`, `notification_router.rs` |
| **LSN-replica CI job (349.3):** a `replica-routing` job runs the three `#[ignore]`d LSN routing tests against a real primary+streaming-standby pair — read-your-writes proven in CI. | `.github/workflows/ci.yml`, `scripts/replica-harness.sh` |
| **MCP projector link tools (349.4):** the six MCP twins of the Cluster-346 REST link routes (Slack + GitHub link/list/unlink); capability-filtered (91 tools). | `crates/maidan-mcp/src/tools/projector.rs`, `contracts/mcp-*.json` |
| **SMTP wire test (349.5):** the real `lettre` `SmtpTransport` proven on the wire against an in-process SMTP sink. | `crates/maidan-server/src/mail.rs` |

Deferred as documented decisions: broad MCP arg-defaulting (declined), cross-crate assembler hoist (declined), README visual media (needs a recorded asset). **The post-flagship audit program (332–349) is complete.**

## v348.0.0 — batch the notification fan-out mute check (audit P2)

| Change | Where |
|--------|-------|
| The follow-up to Cluster 344: a `MessagePosted` fan-out still ran one `is_notification_muted` query per follower (`2 × followers` round-trips). New `Store::filter_muted_members(kind, &[MemberId])` (SQLite dynamic `IN`, Postgres `= ANY`) resolves the muted subset in one query; the fan-out batch-fetches it, meters the suppressed, and writes only the unmuted (concurrently, per 344). `notify`'s insert/email/metric tail extracted into `write_notification` (shared with the mention path). Cuts the fan-out toward `followers + 1` round-trips. A multi-row batch INSERT is a logged further optimization. **Cluster 17 of the post-flagship audit program** | `crates/maidan-store/src/{store.rs,*/notification_prefs.rs,*/mod.rs}`, `crates/maidan-server/src/notification_router.rs` |

## v347.0.0 — projector egress wire-path tests (audit P1.5)

| Change | Where |
|--------|-------|
| The production HTTP clients that build the actual projector-egress request (`SlackWebClient` `chat.postMessage`, `GithubApiClient` issue-comment POST) had no test — the egress tests drive mock sender traits. Added a `with_base_url` constructor to each (production `new` targets the real host) so the wire path is testable, and `egress_wire_e2e` drives the real clients against a loopback recorder: exact URL/headers (bearer + GitHub `User-Agent`)/JSON body + success/error decoding (Slack HTTP-200-`{"ok":false}`; GitHub non-2xx → `Api(status)`). Production behaviour unchanged. **Cluster 16 of the post-flagship audit program** | `crates/maidan-server/src/{slack.rs,github.rs}`, `crates/maidan-server/tests/egress_wire_e2e.rs` |

## v346.0.0 — projector link-management REST surface (audit P2)

| Change | Where |
|--------|-------|
| The Slack/GitHub projectors shipped ingress + egress + a store link table, but no route ever *created* a link — so the link table could never be populated and the projector egress could never fire (a launch feature that couldn't be turned on). New REST surface: `POST`/`GET /workspaces/:wid/slack-links` + `DELETE /…/slack-links/:slack_channel_id`; `POST`/`GET /workspaces/:wid/github-links` + `DELETE /…/github-links?repo=&issue_number=`. The link's `channel_id`/`workspace_id` are derived from `authorize_thread` (can't disagree with the thread); the caller gives only the external id, thread, and attribution member. `POST`/`DELETE`=`workspace:write`, `GET`=`workspace:read`. Full new-route preflight; `projector_links_e2e` proves the created link is what the egress reverse-lookup reads. **Cluster 15 of the post-flagship audit program** | `crates/maidan-server/src/{slack.rs,github.rs,dto.rs,app.rs,openapi/*}`, `contracts/http-capability-map.json` |

## v345.0.0 — MCP `post_message` slash-command parity (audit P2)

| Change | Where |
|--------|-------|
| MCP `post_message` ignored registered slash commands while REST ran them. New dependency-inverted `maidan_mcp::SlashDispatcher` trait (implemented by `maidan-server`'s `ServerSlashDispatcher`, attached to the `McpServer` via `set_slash_dispatcher` in `main.rs` — server-binary only, a `OnceLock` field) lets the MCP post path run slash dispatch when a command is registered, merging the same `{slash_command, slash_response}` metadata as REST (Cluster-211 provisional-insert → dispatch → finalizing-edit shape). The MCP no-slash post was also upgraded to the atomic outbox path (`post_message_with_event` + `publish_stored`). Tests/embedders leave the dispatcher unset → skip slash (no `AppState`↔`McpServer` cycle). **Cluster 14 of the post-flagship audit program** | `crates/maidan-mcp/src/{slash_dispatch.rs,server.rs,tools/message.rs,tools/mod.rs,lib.rs}`, `crates/maidan-server/src/{slash_commands.rs,main.rs}` |

## v344.0.0 — bounded-concurrency notification fan-out (audit P2)

| Change | Where |
|--------|-------|
| The notification router is a serial bus consumer; a `MessagePosted` fanned out to followers in a sequential loop (`2 × followers` store round-trips), so a widely-followed message head-of-line-blocked the whole pipeline. Per-recipient `notify` writes now run with bounded concurrency (`buffer_unordered`, cap 8 — the Cluster-199 pattern) via `fan_out_message_posted`. Behaviour-preserved (same rows; error short-circuits). Batch insert logged as a further optimization. **Cluster 13 of the post-flagship audit program** | `crates/maidan-server/src/notification_router.rs` |

## v343.0.0 — keyset-paginate the channel thread list (audit P2)

| Change | Where |
|--------|-------|
| The last unpaginated list: `GET /channels/:cid/threads` + MCP `list_threads` called unbounded `Store::list_threads(channel_id)`. New `Store::page_threads_for_channel(channel_id, after, limit)` (both backends; keyset `(created_at, id)` ASC, exclusive cursor, `LIMIT` in SQL — channel-scoped twin of `page_threads_for_workspace`) backs `limit` (default 100, clamp 1..=500) + `cursor` on the REST route (`ListThreadsQuery`) and the MCP tool; Postgres routes it via the read replica. Unbounded `list_threads` kept for internal full-list callers. **Cluster 12 of the post-flagship audit program** | `crates/maidan-store/src/{store.rs,sqlite/threads.rs,postgres/threads.rs,*/mod.rs}`, `crates/maidan-server/src/{routes/thread.rs,dto.rs,openapi/paths/api.rs}`, `crates/maidan-mcp/src/tools/{thread.rs,catalog.rs}` |

## v342.0.0 — surface flagship context features to integrators (audit P2)

| Change | Where |
|--------|-------|
| `Integration.md` documented the context pack but omitted the differentiators, so a promoter/integrator couldn't see them. New "Fidelity & context" subsection covers glossary grounding, as-of replay (time travel, `as_of=<event_log_id>`), context snapshots, lean edits, seed/re-ask, and the tool-call transcript — exact wire surface + MCP-tool parity, verified against `dto.rs`/`app.rs`/`catalog.rs`/`mcp-tool-names.json`. Folded a Cluster-341 miss: `Protocols.md` "tool count is 78" → 85. Docs-only. **Cluster 11 of the post-flagship audit program** | `docs/Integration.md`, `docs/Protocols.md` |

## v341.0.0 — docs accuracy reconciliation (audit P2)

| Change | Where |
|--------|-------|
| Audit P2 accuracy fixes, each verified against ground-truth code. **A2A gRPC** reconciled to the honest "partial": `Architecture.md` (implied full parity) + `Protocols.md` ("No gRPC binding" — also wrong) now match `Claims.md` — the gRPC `A2AService` exposes `get_task`/`cancel_task`/`list_tasks` only (verified in `a2a_grpc/mod.rs`); send/push/streaming stay JSON-RPC/REST. **Tool-count drift 78 → 85** in the live integrator docs. **Dead GitHub link** `Capability-Map.md` → `Capability%20Map.md`. **README image pin** `v315` → `v339`. Docs-only. **Cluster 10 of the post-flagship audit program** | `docs/{Architecture,Protocols,Framework Integrations,Adoption}.md`, `examples/README.md`, `README.md` |

## v340.0.0 — fetch-once message authorization (audit P1.4c)

| Change | Where |
|--------|-------|
| The message-keyed twin of 339, completing audit P1.4. ~12 handlers in `message.rs`/`social.rs` called `resolve_message_chain` (get_message + thread + channel) then an access helper that resolved the same chain again + a redundant `ensure_workspace`. New `maidan_auth::authorize_message` resolves `MessageScope {workspace_id, channel_id, thread_id, message_id}` and authorizes in one pass (via `authorize_thread`); `ensure_message_access` delegates to it. Handlers using the scope (edit/tombstone/purge/seed) call `authorize_message`; the rest (votes/reactions/get/edits/mentions) keep `ensure_message_access`. Message-scoped fetches drop ~5→3. Behaviour-identical. **Cluster 9 of the post-flagship audit program** | `crates/maidan-auth/src/{access.rs,lib.rs}`, `crates/maidan-server/src/routes/{message,social}.rs` |

## v339.0.0 — fetch-once thread authorization (audit P1.4b)

| Change | Where |
|--------|-------|
| ~30 thread-scoped handlers double-fetched thread+channel — `resolve_thread_context` (get_thread + get_channel) then `ensure_thread_access` (the same two fetches again) — plus a redundant `ensure_workspace`. New `maidan_auth::authorize_thread` resolves `ThreadScope {workspace_id, channel_id, thread_id}` and authorizes in one fetch; `ensure_thread_access` delegates to it (rule single-sourced; also drops its own duplicate `get_channel`). Handlers that use the scope call `authorize_thread`; the rest keep only `ensure_thread_access`. Behaviour-identical (404 missing / 403 wrong-ws / 403 no-access, same messages); per-request thread+channel fetches halve on that surface. **Cluster 8 of the post-flagship audit program** | `crates/maidan-auth/src/{access.rs,lib.rs}`, `crates/maidan-server/src/routes/{message,thread,social,skills}.rs` |

## v338.0.0 — post-path mention-routing round-trip reduction (audit P1.4a)

| Change | Where |
|--------|-------|
| Every message post (the hottest write path) re-ran `resolve_message_chain` (message→thread→channel→workspace) inside mention routing purely to re-derive a workspace id the caller already had — and did so even for posts with no `@handles`. `publish_routed_mentions` (REST + MCP) now short-circuits on `parse_at_handles(body).is_empty()` (no store work for a plain post) and otherwise routes via `route_mentions_in_message` with the known workspace, dropping the redundant round-trip. Removed the now-unused `route_mentions_for_message`. Behaviour-preserving (mentions still emit `MentionRecorded`). **Cluster 7 of the post-flagship audit program** | `crates/maidan-server/src/routes/mod.rs`, `crates/maidan-mcp/src/tools/message.rs`, `crates/maidan-router/src/{mentions.rs,lib.rs}` |

## v337.0.0 — REST `GET /me` identity endpoint (audit P1.3)

| Change | Where |
|--------|-------|
| The REST twin of Cluster 336's MCP `whoami`, closing agent self-discovery on the HTTP transport. New `GET /me` → `{member_id, workspace_id, capabilities, is_bearer}` reflected from the request's auth (no store access); an agent or `/ui` session with only a base URL + token can discover the `member_id` every member-attributed write requires. `workspace:read`. Full new-route preflight (OpenAPI path + `WhoAmI` schema + capability-map). Audit P1.3 (agent cold-start) now complete across both transports. **Cluster 6 of the post-flagship audit program** | `crates/maidan-server/src/{routes/member.rs,dto.rs,app.rs,openapi/*}`, `contracts/http-capability-map.json` |

## v336.0.0 — agent cold-start: whoami + initialize instructions (audit P1.3)

| Change | Where |
|--------|-------|
| The cheapest adoption unlock: an agent with only a base URL + token couldn't run the hero loop (every hero-loop tool needs its own `member_id`, and MCP `initialize` had no `instructions`). New MCP `whoami` tool → `{member_id, workspace_id, capabilities, is_bearer, bypass}` from auth (`workspace:read`, no store access); `initialize.instructions` now carries a cold-start guide (call `whoami`, then the six-tool hero loop); `AuthContext::capabilities()` accessor. 85 MCP tools. REST `GET /me` twin → Cluster 337. **Cluster 5 of the post-flagship audit program** | `crates/maidan-mcp/src/{tools/whoami.rs,tools/mod.rs,tools/catalog.rs,server.rs}`, `crates/maidan-auth/src/context.rs`, `contracts/mcp-*.json` |

## v335.0.0 — MCP context: batch reads + surface artifacts (audit P1.2)

| Change | Where |
|--------|-------|
| The MCP context assembler had a per-message N+1 (refs + edits fetched per message) and omitted artifacts; the REST one batched both + included artifacts. Now `get_thread_context`/`get_thread_context_as_of` use batched shared helpers (`collect_references` `src_id=ANY`, `collect_edit_views` with optional as-of cutoff, `collect_artifacts`) and surface an `artifacts` array — matching REST. Sha extractor shared via `maidan_types::artifact_shas_from_metadata`. REST unchanged (query-count guard green). Full cross-crate assembler hoist deferred (maidan-router `ThreadContext` name collision + utoipa/futures plumbing; maintainability-only, message fold already shared). **Cluster 4 of the post-flagship audit program** | `crates/maidan-types/src/models.rs`, `crates/maidan-mcp/src/context.rs`, `crates/maidan-server/src/thread_context.rs` |

## v334.0.0 — MCP write-path event parity, the rest (audit P1.1b)

| Change | Where |
|--------|-------|
| The 7 remaining event-less MCP write tools now emit domain events (via `McpServer::publish_stored`): `cast_vote`/`add_reaction`/`remove_reaction`/`pin_message`/`unpin_message`/`add_reference` → `*_with_event`; `record_mention` → `record_mention_with_event`; and MCP `post_message`/`post_dm_message` publish `MentionRecorded` per @mentioned member (a shared `publish_routed_mentions` helper). MCP mutations now reach WS/SSE, at-least-once, federation, and the notification router / `wait_for_mention` like REST. **P1.1 (MCP write-path parity) complete** (333 edit + 334 rest). **Cluster 3 of the post-flagship audit program** | `crates/maidan-mcp/src/{tools/social.rs,tools/reference.rs,tools/message.rs,tools/mod.rs}` |

## v333.0.0 — MCP edit_message emits MessageEdited (audit P1.1a)

| Change | Where |
|--------|-------|
| Correctness fix (post-flagship audit P1.1a): MCP `edit_message` was event-less (`store.edit_message`), so an MCP edit appended no `MessageEdited` → the flagship as-of replay returned the stale body forever and the embedding indexer never reindexed. Now it calls `edit_message_with_event` (atomic row + event) and the new `McpServer::publish_stored` bus-notify → as-of replay, reindex, and WS/SSE + notification-router all see MCP edits, matching REST. `publish_stored` is the reusable seam for the rest of the MCP write-path migration (Cluster 334). **Cluster 2 of the post-flagship audit program** | `crates/maidan-mcp/src/{server.rs,tools/message.rs,tools/mod.rs}` |

## v332.0.0 — MCP artifact tenant isolation (audit P0.1)

| Change | Where |
|--------|-------|
| Security fix (post-flagship audit P0.1): the MCP artifact tools now enforce Cluster-204 cross-tenant isolation. `get_artifact_metadata` + the `maidan://artifacts/{sha}` resource read gate on `artifact_ref_exists(auth.workspace_id, sha)` → `NotFound` when absent (no cross-tenant oracle, matching REST); MCP uploads record the per-workspace ref via `record_artifact_ref`; `resources::read` uses `size_bytes` metadata instead of loading the blob. **Cluster 1 of the post-flagship audit program** | `crates/maidan-mcp/src/{tools/artifact.rs,tools/mod.rs,resources.rs,server.rs}` |

## v331.0.0 — flagship arc closeout (decision)

| Change | Where |
|--------|-------|
| Docs-only closeout of the fidelity + context flagship arc (319–331). A "Product scope" ADR records the arc complete and **declines** its optional tail (seed `pack`/`prefix` inclusion, a `WorkSeeded` event, the flow/setup template) as composable from shipped primitives — declined, not deferred, with revisit conditions. Open Work / Roadmap marked complete. Clean baseline for a research round. **Cluster 13 (closeout) of the fidelity + context flagship arc** | `docs/Decisions.md`, `docs/Open Work.md`, `docs/Roadmap.md` |

## v330.0.0 — context snapshot MCP tool (flagship arc)

| Change | Where |
|--------|-------|
| MCP `snapshot_thread_context` — the twin of the 329 REST route: freeze the assembled context pack (live or `as_of`) into the content-addressed artifact store, returning the `Artifact` (`kind=context_snapshot`). `artifact:upload`; reuses `context::get_thread_context` + the modern `upsert_artifact_with_event` + Cluster-204 ref + bus-notify (an MCP-frozen snapshot is fetchable by its workspace, unlike the older MCP artifact tools). Both contracts → 84 tools. Context snapshot is now complete over REST + MCP. **Cluster 12 of the fidelity + context flagship arc** | `crates/maidan-mcp/src/tools/{snapshot.rs,mod.rs,catalog.rs}`, `contracts/mcp-*.json` |

## v329.0.0 — immutable context snapshot artifact (flagship arc)

| Change | Where |
|--------|-------|
| `POST /threads/:id/context/snapshot` freezes the assembled context pack (live or `as_of`) into the existing content-addressed artifact store — a tamper-evident, deduped record of exactly what the agent was handed (identical packs share a blob). Returns the `Artifact` (`kind=context_snapshot`, `application/json`); fetchable at `GET /artifacts/:sha`; gated `artifact:upload` + thread access. New `ArtifactKind::ContextSnapshot` + migration pg `0055` / sqlite `0054` widening the artifact-kind `CHECK`. Reuses the artifact store wholesale (no new blob path). **Cluster 11 of the fidelity + context flagship arc** | `crates/maidan-types/src/models.rs`, `crates/maidan-server/src/{routes/thread.rs,app.rs,openapi/*}`, `migrations/{postgres/0055,sqlite/0054}_artifact_kind_context_snapshot.sql`, `contracts/http-capability-map.json` |

## v328.0.0 — seed-from-message MCP tool (flagship arc)

| Change | Where |
|--------|-------|
| MCP `seed_from_message` — the twin of the 327 REST route: `{message_id, title, inclusion?, channel_id?}` spawns a titled child thread + a `seeded_from` reference edge (+ a quoting first message for `inclusion=quote`). `workspace:write`; source access via the pre-dispatch gate, target channel checked in-handler. Uses `*_with_event` store methods + a bus-notify of the returned event (atomic log + real-time parity — the MCP analogue of REST `publish_stored`; the first MCP thread-creating tool). Both contracts → 83 tools. **Cluster 10 of the fidelity + context flagship arc** | `crates/maidan-mcp/src/tools/{seed.rs,mod.rs,catalog.rs}`, `contracts/mcp-*.json` |

## v327.0.0 — seed-from-message (flagship arc)

| Change | Where |
|--------|-------|
| The write side of "re-ask": `POST /messages/:id/seed` spawns a titled, claimable child thread from a source message, linked by a `seeded_from` reference edge (new thread → source). `inclusion`: `pointer` (default, edge only) or `quote` (first message quotes the source). Source untouched; N seeds per source; gated `workspace:write` + source read + target-channel write. Reuses existing primitives — no bespoke table, no new event kind (emits `ThreadCreated` + `ReferenceAdded`); lineage is queryable via the Cluster-320 reverse reference query. New `RelationKind::SeededFrom` (controlled vocab → 8). MCP tool follows in 328. **Cluster 9 of the fidelity + context flagship arc** | `crates/maidan-types/src/models.rs`, `crates/maidan-server/src/{routes/message.rs,dto.rs,app.rs,openapi/*}`, `contracts/http-capability-map.json` |

## v326.0.0 — as-of context replay (flagship arc)

| Change | Where |
|--------|-------|
| `GET /threads/:id/context?as_of=<event_id>` (+ MCP `get_thread_context` `as_of` arg) reconstructs a thread as it stood at that event-log id — deterministic over the immutable log, no fresh search. A since-edited message shows its as-of body; a since-tombstoned message reappears (both impossible from current rows). `Store::list_thread_events_through` (both backends) + shared `maidan_types::reconstruct_messages_through` fold `MessagePosted`/`MessageEdited` (full `Message` payloads) + `MessageTombstoned`; additive components cut by the anchor's time; glossary omitted. Serves audit + re-ask-from-before-a-tangent. Unknown id → `404`. **Cluster 8 of the fidelity + context flagship arc** | `crates/maidan-store/src/{store.rs,{postgres,sqlite}/{events,mod}.rs}`, `crates/maidan-types/src/events.rs`, `crates/maidan-server/src/{thread_context.rs,dto.rs,routes/{thread,workspace}.rs}`, `crates/maidan-mcp/src/{context.rs,tools/catalog.rs}` |

## v325.0.0 — agent conventions: decisions, supersession, acks (flagship arc)

| Change | Where |
|--------|-------|
| The "near-zero-code conventions" half of the arc's confidence-and-conventions item — codified as docs with a convention-proving e2e and **zero new server code** ("a room, not a brain"). `docs/Integration.md` "Agent conventions" documents: **decision records** (ADR-shaped `thread_result` JSON), **supersession** (a `supersedes` reference edge + `status` flip; `GET /references?dst_kind=…&relation=supersedes` = "what replaced this?"), and **grounding acks** (an `ack` vote grounding a message as of its `created_at`, detectably stale once edited later). `decision_convention_e2e` proves the whole trio over the real HTTP API. **Cluster 7 of the fidelity + context flagship arc** | `docs/Integration.md`, `crates/maidan-server/tests/decision_convention_e2e.rs` |

## v324.0.0 — optional vote confidence (flagship arc)

| Change | Where |
|--------|-------|
| An optional `confidence` weight (0..1) on a vote, so consumers can compute weighted consensus instead of a flat tally. `maidan_votes.confidence` (pg `0054` / sqlite `0053`, nullable); `Vote`/`NewVote` gain `confidence: Option<f64>` (omitted when absent); REST `POST/GET /messages/:id/votes` + MCP `cast_vote`; range validated at the API edge. Re-casting the same `(message, member, kind)` upserts the confidence (count idempotent). **Cluster 6 of the fidelity + context flagship arc** | `migrations/{postgres/0054,sqlite/0053}_vote_confidence.sql`, `crates/maidan-types/src/models.rs`, `crates/maidan-store/src/{postgres,sqlite}/votes.rs`, `crates/maidan-server/src/{dto.rs,routes/social.rs}`, `crates/maidan-mcp/src/tools/{social,catalog}.rs` |

## v323.0.0 — glossary in the context pack (flagship arc)

| Change | Where |
|--------|-------|
| The grounding payoff: `GET /threads/:id/context` + `GET /workspaces/:wid/context` (REST) and the `get_thread_context`/`get_workspace_context` MCP tools now carry a `glossary` field, so an agent's context is grounded in the workspace's shared vocabulary without a second call. New `include_glossary` param, **default `true`**; `skip_serializing_if` empty (byte-neutral when no glossary); the workspace pack carries it once at the top (not repeated per nested thread — `build_workspace_context` dedups). One constant query per pack, so the context query-count independence invariant is unchanged. **Cluster 5 of the fidelity + context flagship arc — the glossary layer (321→322→323) is complete** | `crates/maidan-server/src/{thread_context.rs,dto.rs,routes/{thread,workspace}.rs}`, `crates/maidan-mcp/src/{context.rs,tools/catalog.rs}` |

## v322.0.0 — glossary REST + MCP (flagship arc)

| Change | Where |
|--------|-------|
| The 321 glossary, surfaced: REST `PUT/GET/DELETE /workspaces/:wid/glossary/:term` + `GET /workspaces/:wid/glossary` (list), and MCP `set_glossary_term`/`get_glossary_term`/`list_glossary_terms`. Agents can define, look up, and list a workspace's canonical `term -> definition`. `set` upserts (`workspace:write`, `created_by` = acting member); reads are `workspace:read`; `delete` stays REST-only (the 220/229 precedent). **Cluster 4 of the fidelity + context flagship arc** | `crates/maidan-server/src/{routes/glossary.rs,dto.rs,app.rs,openapi/*}`, `crates/maidan-mcp/src/tools/{glossary.rs,mod.rs,catalog.rs}`, `contracts/{http-capability-map,mcp-*}.json` |

## v321.0.0 — shared glossary foundation (flagship arc)

| Change | Where |
|--------|-------|
| A workspace's canonical `term -> definition` (+ aliases) so agents use words the same way — the anti-drift pin and the target of 319's `defines` reference relation. `maidan_glossary_terms` (pg `0053` / sqlite `0052`, `UNIQUE(workspace_id, term)`, aliases as JSONB/TEXT-JSON), `GlossaryTerm`/`NewGlossaryTerm` models, and `Store::{set,get,list,delete}_glossary_term` (both backends; `set` upserts, preserving authorship + bumping `updated_at`). Flat by design — hierarchy is a knowledge-graph product line, out of scope. **Zero-blast-radius store foundation** — no routes/tools yet (322). **Cluster 3 of the fidelity + context flagship arc** | `migrations/{postgres/0053,sqlite/0052}_glossary_terms.sql`, `crates/maidan-types/src/models.rs`, `crates/maidan-store/src/{store.rs,migrate.rs,{postgres,sqlite}/{glossary,mod}.rs}` |

## v320.0.0 — reverse-edge + by-type reference queries (flagship arc)

| Change | Where |
|--------|-------|
| The traversal payoff for 319's typed relations: `Store::list_references_to` (reverse edge, reuses the existing `idx_references_dst` index — no migration); `GET /references` reshaped to query FROM a source or TO a target + optional `relation` filter (exactly one pair, anchor-gated, same route/cap); new MCP `list_references` tool (MCP could add but not list references). "What refutes X / what references this" is now queryable. **Cluster 2 of the fidelity + context flagship arc** | `crates/maidan-store/src/{store.rs,{postgres,sqlite}/{refs,mod}.rs}`, `crates/maidan-server/src/{dto.rs,routes/reference.rs}`, `crates/maidan-mcp/src/tools/{reference.rs,mod.rs,catalog.rs}`, `contracts/mcp-*.json` |

## v319.0.0 — typed reference relations (flagship arc keystone)

| Change | Where |
|--------|-------|
| `Reference.relation` is now a controlled `RelationKind` (`supports/refutes/defines/depends/duplicates/grounds/supersedes` + `Other(String)` escape) instead of a free string — the same subject→predicate→object shape as IBIS/PROV/ClaimReview, turning the reference graph into a machine-navigable argument/provenance graph. Serializes as the bare snake_case string (wire byte-identical); both store backends bind `as_str()`/parse `from_wire`, column stays TEXT (no migration); REST `CreateReference` + MCP `add_reference` inputs typed; OpenAPI/MCP schemas unchanged (`string`). **Cluster 1 of the fidelity + context flagship arc.** No backwards-compat shim (pre-launch) | `crates/maidan-types/src/models.rs`, `crates/maidan-store/src/{postgres,sqlite}/{refs,import}.rs`, `crates/maidan-server/src/dto.rs`, `crates/maidan-mcp/src/tools/reference.rs` |

## v318.0.0 — token-pack evidence

| Change | Where |
|--------|-------|
| A number for the "far fewer tokens" claim: `token_pack` measures the scoped context pack vs dumping the whole channel — **~6.8× fewer tokens** (in-process SQLite, 8×40 msgs; scoped pack ~4 951 vs naive ~33 908 tokens), plus ~1.3× from lean edits. Bytes exact, `≈chars/4` tokens, ratio tokenizer-independent; `#[ignore]`d harness + pure estimator unit-tested in CI. `Benchmark.md` gained a "Context-pack token savings" section; `Claims.md` token row → "Shipped + measured" with the evidence link. **Closes the launch-prep leg of the 2026-08-28 sweep (315–318)** | `crates/maidan-server/tests/token_pack.rs`, `docs/Benchmark.md`, `docs/Claims.md` |

## v317.0.0 — Bet 2 MCP snippet pack + two-language lease demo

| Change | Where |
|--------|-------|
| The falsifiable hello-world: a Python SDK worker + a TypeScript SDK worker both `claim_next_thread` on one channel → Maidan hands each task to exactly one (no cross-language double-claim; drained queue → `null`; no LLM); verified end-to-end via `scripts/lease-demo.sh`. MCP client configs for Cursor/Claude (`/mcp/streamable`, bearer, `2026-07-28`). LangChain/AutoGen examples now **filter to the six-tool hero loop** (client-side; catalog stays 78, 8-seam callable) instead of loading all ~78. CI guards the new scripts/configs | `examples/lease_demo/`, `scripts/lease-demo.sh`, `examples/{cursor-mcp,claude-desktop-mcp}.json`, `examples/{langchain,autogen,rest}_maidan.py`, `examples/README.md`, `docs/Framework Integrations.md`, `.github/workflows/ci.yml` |

## v316.0.0 — docs honesty scrub + honest prebuilt-image path

| Change | Where |
|--------|-------|
| Corrected every verified stale/false doc at v315 (Claims.md A2A-gRPC overclaim → "gRPC = task read/cancel/list, no SendMessage"; `mail.rs`/`server.rs`/`Framework Integrations`/`Threat-Model`/`sdk/README`/`Clients`/`Client Testing`/`Promotion`/`AGENTS`/`Integration`/`CLAUDE`/`SECURITY` staleness; README "experimental A2A"→"A2A v1.0"); fixed two more won't-boot commands (`introduction.md` `cargo run` missing session secret; `Pi.md` `docker run` missing the AUTH_DISABLED ack). Added an honest README "Prebuilt image (no clone)" note — **smoke found the planned `docker run … maidan init` impossible** (prod image is distroless, no CLI/shell), so a true one-command no-clone eval is deferred (needs the quickstart image on GHCR). Published the stuck `v300` release draft | `docs/{Claims,Framework Integrations,Threat-Model,Pi,Integration,Clients,Client Testing,Promotion}.md`, `book/src/introduction.md`, `README.md`, `AGENTS.md`, `CLAUDE.md`, `SECURITY.md`, `sdk/README.md`, `crates/maidan-server/src/mail.rs`, `crates/maidan-mcp/src/server.rs` |

## v315.0.0 — pre-launch correctness & DX + research-sweep fold

| Change | Where |
|--------|-------|
| `hash-v1` embedding default warns at boot ("not semantically meaningful; set `MAIDAN_EMBEDDING_PROVIDER`") so a stranger isn't silently served near-random "semantic" hits; fixed the README no-Docker `MAIDAN_SESSION_SECRET` (was 28 bytes, needs ≥32); `event_stream` replay logs a failed delivery-cursor advance instead of `let _ =`; defensive `ensure_acting_member` on the legacy `/members/:id/mentions`+`/inbox` handlers (the audit's "session can read another's inbox" was a **false positive** — bearer-only routes, no `/ui/api` mount; guards future-proof a later mount). Folded the 2026-08-28 research sweep into Open Work (v314 currency + 315–318 + the fidelity/context flagship arc + anti-goals) | `crates/maidan-server/src/{main.rs,event_stream.rs,routes/member.rs}`, `README.md`, `crates/maidan-server/tests/ui_channels_e2e.rs`, `docs/Open Work.md` |

## v314.0.0 — launch honesty: claims sheet, policies, release verification

| Change | Where |
|--------|-------|
| Fixed the README headline one-liner (didn't boot: auth on needs a ≥32-byte `MAIDAN_SESSION_SECRET`); published an honest claims sheet mapping every README/site claim → a gate/test/"not yet" (`docs/Claims.md`, on the site + linked from README); added copy-paste keyless-cosign release verification (`SECURITY.md#verifying-a-release`) + a human `CHANGELOG-highlights.md` with a Release-notes template; reconciled `CONTRIBUTING.md` to the solo-maintained/admin-merge/8-required-checks model (Launch L3/L4/L6 + Pre-Public Hardening F2/G5) | `README.md`, `docs/Claims.md`, `SECURITY.md`, `CONTRIBUTING.md`, `CHANGELOG-highlights.md`, `book/src/SUMMARY.md`, `book/sync-docs.sh` |

## v313.0.0 — default-secure quickstart (launch hardening F4)

| Change | Where |
|--------|-------|
| The quickstart happy path is token-based, not `AUTH_DISABLED` (Pre-Public Hardening F4 / Launch L1): `compose.quickstart.yaml` runs auth ON (dev `MAIDAN_SESSION_SECRET` + `MAIDAN_BOOTSTRAP=1`), the README mints a bearer token via `maidan init` and runs the two-agent demo with it, and `scripts/quickstart-two-agents.sh` is auth-aware (`MAIDAN_TOKEN`/`MAIDAN_WORKSPACE`). New `compose.quickstart.insecure.yaml` override demotes `AUTH_DISABLED` to a clearly-labelled local-only appendix. Quickstart image bumped `v277.0.0`→`v312.0.0` (re-pinned tarball SHAs; `maidan init` landed in `v279`). Both paths validated end-to-end; CI validates both compose files | `compose.quickstart.yaml`, `compose.quickstart.insecure.yaml`, `docker/Dockerfile.quickstart`, `scripts/quickstart-two-agents.sh`, `README.md`, `docs/Integration.md`, `.github/workflows/ci.yml` |

## v312.0.0 — GitHub projector egress (arc closer)

| Change | Where |
|--------|-------|
| GitHub egress: `GithubSender` trait + `GithubApiClient` (`POST /repos/{repo}/issues/{n}/comments`, bearer + `User-Agent` + `Accept: application/vnd.github+json`); `route_message_to_github` relays a linked-thread Maidan message to a GitHub issue/PR comment, skipping GitHub-sourced messages (`metadata.github`) for loop safety; hooked into the notification-router `MessagePosted` path beside the Slack egress. `AppState.github_sender`/`attach_github_sender`; `get_github_issue_link_by_thread` store lookup; `maidan_github_egress_total` metric. **Completes the bidirectional GitHub projector (310–312)** and the projector arc (Slack 307–309 + Git 310–312) | `crates/maidan-server/src/{github.rs,notification_router.rs,state.rs,main.rs}` |

## v311.0.0 — GitHub projector: issue links + inbound routing

| Change | Where |
|--------|-------|
| `maidan_github_issue_links` table (pg 0052 / sqlite 0051; PK `(repo, issue_number)`) + `GithubIssueLink` model + store (both backends: link/get/by-thread/list/unlink) — maps a GitHub issue/PR → Maidan channel/thread/member. `github.rs` routes an inbound `issue_comment` on a linked issue into the mapped thread (`"{login}: {body}"`); skips `Bot` comments + stamps `metadata.github` for loop prevention | `crates/maidan-store/src/{postgres,sqlite}/github_links.rs`, `crates/maidan-server/src/github.rs`, `crates/maidan-types/src/models.rs` |

## v310.0.0 — GitHub projector ingress foundation

| Change | Where |
|--------|-------|
| Config-gated GitHub projector ingress (a projector, not a bot): `POST /integrations/github/events` (unauthed; GitHub signs `X-Hub-Signature-256`) — signature verification (reuses `webhooks::verify_signature`; GitHub's `sha256=hex(HMAC)` == Maidan's own scheme) + the `ping` setup handshake; `404` when unconfigured, `401` on bad signature. `GithubConfig::from_env` (`MAIDAN_GITHUB_*`) + `AppState.github`/`attach_github` | `crates/maidan-server/src/{github.rs,app.rs,state.rs,main.rs}` |

## v309.0.0 — Slack projector egress (arc closer)

| Change | Where |
|--------|-------|
| Slack egress: `SlackSender` trait + `SlackWebClient` (`chat.postMessage`); `route_message_to_slack` relays a linked-thread Maidan message to Slack, skipping Slack-sourced messages (`metadata.slack`) for loop safety; hooked into the notification-router `MessagePosted` path. `AppState.slack_sender`/`attach_slack_sender`; `get_slack_channel_link_by_thread` store lookup; `maidan_slack_egress_total` metric. **Completes the bidirectional Slack projector (307–309)** | `crates/maidan-server/src/{slack.rs,notification_router.rs,state.rs,main.rs}`, `crates/maidan-store/src/{postgres,sqlite}/slack_links.rs` |

## v308.0.0 — Slack projector: channel links + inbound routing

| Change | Where |
|--------|-------|
| `maidan_slack_channel_links` table (pg 0051 / sqlite 0050) + `SlackChannelLink` model + store (both backends: link/get/list/unlink) — maps a Slack channel → Maidan channel/thread/member. `slack.rs` routes an inbound Slack `message` in a linked channel into the mapped thread (`"{user}: {text}"` via `post_message_with_event`); skips bot/subtype + stamps `metadata.slack` for loop prevention | `crates/maidan-store/src/{postgres,sqlite}/slack_links.rs`, `crates/maidan-server/src/slack.rs`, `crates/maidan-types/src/models.rs` |

## v307.0.0 — Slack projector ingress foundation

| Change | Where |
|--------|-------|
| Config-gated Slack projector ingress (a projector, not a bot — no LLM in Maidan): `POST /integrations/slack/events` (unauthed; Slack signs its own requests) — signature verification (`v0` HMAC-SHA256, ±5-min replay, constant-time) + the Events-API `url_verification` handshake; `404` when unconfigured, `401` on bad signature. `SlackConfig::from_env` (`MAIDAN_SLACK_*`) + `AppState.slack`/`attach_slack` | `crates/maidan-server/src/{slack.rs,app.rs,state.rs,main.rs}` |

## v306.0.0 — mail DLQ ops (arc closer)

| Change | Where |
|--------|-------|
| `GET /operator/mail/dead` + `POST /operator/mail/dead/{id}/requeue` (`token:admin`) — list dead-lettered notification emails (`DeadMail` view) + requeue one for retry (resets to pending, attempts cleared). Store `list_dead_mail`/`requeue_dead_mail` both backends. **Closes the durable-mail-retry arc (304→306)** | `crates/maidan-server/src/routes/mail_ops.rs`, `crates/maidan-store/src/{postgres,sqlite}/mail_outbox.rs`, `crates/maidan-types/src/models.rs`, `contracts/http-capability-map.json` |

## v305.0.0 — mail-outbox worker + router enqueue

| Change | Where |
|--------|-------|
| Notification email is durable: the router `enqueue_mail`s (after its suppression checks) instead of a best-effort inline send; a new `mail_worker` background loop drains the outbox with retry (exp backoff 30s→1h) + dead-lettering (8 attempts). Spawned when a transport is configured; tick via `MAIDAN_MAIL_WORKER_TICK_SECS` (default 5s). Multi-replica-safe. Metric outcomes `enqueued`/`sent`/`retry`/`dead` | `crates/maidan-server/src/{mail_worker.rs,notification_router.rs,main.rs}` |

## v304.0.0 — durable mail outbox foundation

| Change | Where |
|--------|-------|
| `maidan_mail_outbox` table (pg 0050 / sqlite 0049) + `MailOutbox`/`NewMailOutbox`/`MailOutboxId` + store (both backends): `enqueue_mail`, `claim_next_due_mail` (atomic leased claim — `FOR UPDATE SKIP LOCKED` / serialized tx; bumps attempts + leases forward), `mark_mail_delivered`, `mark_mail_failed` (reschedule or dead-letter), `count_dead_mail`. Zero-blast-radius foundation for the durable notification-email retry queue | `migrations/{postgres/0050,sqlite/0049}_mail_outbox.sql`, `crates/maidan-store/src/{postgres,sqlite}/mail_outbox.rs`, `crates/maidan-types/src/{models,ids}.rs` |

## v303.0.0 — advertise MCP `2026-07-28` (arc closer)

| Change | Where |
|--------|-------|
| MCP default flipped to `2026-07-28` (`DEFAULT_PROTOCOL_VERSION`); version-less clients negotiate it, explicit `2024-11-05` still honored. Federation card reports `preferred_protocol_version()`; MCP reference + crate doc describe 2026 (stateless + routing headers). **Closes the MCP `2026-07-28` arc (300–303)** | `crates/maidan-mcp/src/{server.rs,reference.rs,lib.rs}`, `crates/maidan-server/src/federation.rs` |
| `Integration.md` + `Protocols.md` advertise `2026-07-28` (banner/transport table/how-to/decision tree/J-rows); J2 "temporary honesty" retired | `docs/Integration.md`, `docs/Protocols.md` |

## v302.0.0 — MCP `2026-07-28` routing headers

| Change | Where |
|--------|-------|
| SEP-2243 `Mcp-Method` / `Mcp-Name` routing headers on `POST /mcp` + `/mcp/streamable` — optional, but when present must match the body (`Mcp-Method`==method, `Mcp-Name`==tool/prompt name or resource uri) else `400`, so a gateway can route/authorize without parsing JSON (`validate_routing_headers`). Batches skip it; a stray `Mcp-Name` on an unnamed method is ignored | `crates/maidan-server/src/{mcp.rs,mcp_streamable.rs}` |

## v301.0.0 — MCP `2026-07-28` stateless streamable core

| Change | Where |
|--------|-------|
| `POST /mcp/streamable` serves a `2026-07-28` request statelessly — inline JSON-RPC, **no `Mcp-Session-Id` minted or required**, regardless of `Accept` (sessions removed in the revision; `is_stateless_request`/`STATELESS_PROTOCOL_VERSION`). The `2024-11-05` SSE-session path is unchanged; live-wait + server→client stay on `GET /mcp/stream`/WS/`wait_for_*` (J3.4). `POST /mcp` was already stateless | `crates/maidan-server/src/{mcp.rs,mcp_streamable.rs}` |

## v300.0.0 — MCP `2026-07-28` version negotiation

| Change | Where |
|--------|-------|
| MCP `initialize` + the `MCP-Protocol-Version` header now negotiate `2026-07-28` additively (`SUPPORTED_PROTOCOL_VERSIONS = ["2026-07-28","2024-11-05"]`); `preferred_protocol_version()` returns a new explicit `DEFAULT_PROTOCOL_VERSION` held at `2024-11-05` so version-less/older clients are unchanged. Opens the J3 arc; default-flip + advertising deferred until the stateless-core + routing headers land | `crates/maidan-mcp/src/server.rs` |

## v299.0.0 — SDK interop CI

| Change | Where |
|--------|-------|
| Report-only `sdk-interop` CI job: boots a source-built server (SQLite, auth disabled) and runs all four SDK black-box suites against it (`scripts/sdk-test.sh` ts→py→go→rust; four toolchains, server build warmed once). `continue-on-error`, not required — proves the clients interop without blocking merges. Closes the SDK loop (294–299) | `.github/workflows/ci.yml` |

## v298.0.0 — SDK release workflow

| Change | Where |
|--------|-------|
| Publish the four SDKs to their registries on per-language tags (`sdk-ts/py/rs/go-vX.Y.Z` → npm/PyPI/crates.io/`sdk/go/vX.Y.Z` re-tag); per-job version guard (tag must match manifest); auth via `NPM_TOKEN`/`PYPI_TOKEN`/`CRATES_TOKEN` repo secrets. All four verified publish-ready by local dry-run | `.github/workflows/sdk-release.yml`, `docs/SDK Release.md` |
| Gitignore `release_secrets.txt` (never commit tokens) + `sdk/python/.gitignore`; npm `repository.url` polish | `.gitignore`, `sdk/python/.gitignore`, `sdk/typescript/package.json` |

## v297.0.0 — Rust SDK (0.1.0), SDK arc finale

| Change | Where |
|--------|-------|
| Fourth/final usable language client, to the frozen v1 contract; a **standalone crate** (no `maidan-*` dependency). Service-handle surface (`workspaces()`/`channels()`/`threads()`/`messages()`/`artifacts()`), `claim_next_thread`/`renew_claim`, `subscribe` + `wait_for_{result,mention,ready}`, `MaidanError` (status/body/retry_after, is_conflict/is_forbidden/is_rate_limited/is_transport), responses as `serde_json::Value`, `client.mcp_url` string. Small sync stack (`ureq`+`tungstenite`+`serde_json`; std has no HTTP/TLS). 0.1.0 | `sdk/rust/{Cargo.toml,src/lib.rs,src/subscribe.rs,README.md}` |
| `cargo test` black-box suite (5/5: hero loop, claim-next, error surfacing, WS subscribe) via the Cluster-294 harness (`scripts/sdk-test.sh rust`); `clippy -D warnings` + `fmt` clean. **Completes the SDK arc (294–297): TS, Python, Go, Rust at 0.1.0** | `sdk/rust/tests/black_box.rs`, `scripts/sdk-test.sh` |

## v296.0.0 — Go SDK (0.1.0)

| Change | Where |
|--------|-------|
| Third usable language client, to the frozen v1 contract, **dependency-free (stdlib only)**: REST via `net/http`; `Subscribe` via a small hand-rolled RFC-6455 WebSocket client. Service-struct surface (`Workspaces`/`Channels`/`Threads`/`Messages`/`Artifacts`), `ClaimNextThread`/`RenewClaim`, `Subscribe` + `WaitFor{Result,Mention,Ready}`, `APIError` (Status/Body/RetryAfter, IsConflict/IsForbidden/IsRateLimited), `c.MCPURL` string. Responses as `maidan.M` (unknown fields ignored). 0.1.0 | `sdk/go/{client.go,ws.go,README.md}` |
| `go test` black-box suite (hero loop, claim-next, error surfacing, WS subscribe) via the Cluster-294 harness (`scripts/sdk-test.sh go`); `go vet` + `gofmt` clean | `sdk/go/client_test.go`, `scripts/sdk-test.sh` |

## v295.0.0 — Python SDK (0.1.0)

| Change | Where |
|--------|-------|
| Second usable language client, to the frozen v1 contract, **dependency-free (stdlib only)**: REST via `urllib`; `subscribe` via a small hand-rolled RFC-6455 WebSocket client. snake_case surface (`workspaces`/`channels`/`threads`/`messages`/`artifacts`), `claim_next_thread`/`renew_claim`, `subscribe` + `wait_for_{result,mention,ready}`, `MaidanError` (status/body/retry_after, is_conflict/is_forbidden/is_rate_limited), `client.mcp_url` string. Bumped 0.0.1 → 0.1.0 | `sdk/python/{src/maidan/,pyproject.toml,README.md}` |
| `pytest` black-box suite (5/5 pass: hero loop, claim-next, error surfacing, WS subscribe) run via the Cluster-294 harness (`scripts/sdk-test.sh python`) | `sdk/python/tests/test_client.py`, `scripts/sdk-test.sh` |

## v294.0.0 — TypeScript SDK (0.1.0)

| Change | Where |
|--------|-------|
| First usable language client, to the frozen v1 contract: a dependency-free `Client` (REST + WebSocket) with namespaced methods (`workspaces`/`channels`/`threads`/`messages`/`artifacts`), `claimNextThread`/`renewClaim`, `subscribe` + `waitFor{Result,Mention,Ready}`, `MaidanError` (status/body/retryAfter, isConflict/isForbidden/isRateLimited), full `.d.ts` types (branded IDs), `client.mcpUrl` string. Bumped 0.0.1 → 0.1.0 | `sdk/typescript/{index.js,index.d.ts,package.json,README.md}` |
| Language-agnostic SDK black-box test harness (build + boot SQLite server + run suite + teardown) + a `node --test` TS suite (5/5 pass: hero loop, claim-next, error surfacing, WS subscribe) | `scripts/sdk-test.sh`, `sdk/typescript/test.mjs` |

## v289.0.0 — A2A interop conformance (compliance arc finale)

| Change | Where |
|--------|-------|
| A2A conformance client (`examples/a2a_interop.py`, httpx): validates the Agent Card §4.4.1 + JSON-RPC + REST bindings against the spec. Harness `scripts/a2a-interop.sh` (boot + run + teardown) + a report-only `a2a interop` CI job. Live-verified. **Completes the A2A v1.0 arc (282–289): all three transports + negotiation** | `examples/a2a_interop.py`, `scripts/a2a-interop.sh`, `.github/workflows/ci.yml`, `docs/Framework Integrations.md` |

## v288.0.0 — A2A transport negotiation + configurable origin (compliance arc, part 7)

| Change | Where |
|--------|-------|
| Agent Card advertises transports configurably (§5.2): `MAIDAN_A2A_PUBLIC_ORIGIN` → absolute HTTP interface URLs; `MAIDAN_A2A_GRPC_PUBLIC_ADDR` → a `GRPC` `AgentInterface`. Config in `AppState`, threaded through the well-known card + `GetExtendedAgentCard`. Default card unchanged. Production.md documents A2A deployment | `crates/maidan-server/src/{a2a_agent.rs,state.rs,main.rs}`, `docs/Production.md` |

## v287.0.0 — A2A gRPC binding (compliance arc, part 6)

| Change | Where |
|--------|-------|
| A2A gRPC binding (§10): tonic `A2AService` (GetTask/CancelTask/ListTasks) on a config-gated port (`MAIDAN_A2A_GRPC_ADDR`), thin adapters over the shared ops; auth from gRPC metadata. Vendored codegen (minimal self-contained proto → local `tonic-prost-build` → committed `generated.rs`, no build-time protoc). Off by default | `crates/maidan-server/src/a2a_grpc/{mod.rs,generated.rs}`, `crates/maidan-server/proto/a2a.proto`, `crates/maidan-server/src/main.rs`, `crates/maidan-server/Cargo.toml` |
| deny.toml quarantines tonic-server's axum 0.8 duplicate (skip-tree `axum@0.8.9`) | `deny.toml` |

## v286.0.0 — A2A HTTP+JSON/REST binding (compliance arc, part 5)

| Change | Where |
|--------|-------|
| A2A REST binding (§11): 9 request/response routes under `/a2a/v1` (`message:send`, `tasks`, `tasks/{id}`, `tasks/{id}:cancel`, push-config CRUD, `extendedAgentCard`) as thin adapters over the JSON-RPC ops (`rest_response` result/error→HTTP). Agent Card advertises the HTTP+JSON interface. Streaming REST deferred | `crates/maidan-server/src/{a2a_agent.rs,app.rs}`, `contracts/http-capability-map.json` |

## v285.0.0 — A2A Agent Card §4.4.1 schema (compliance arc, part 4)

| Change | Where |
|--------|-------|
| Agent Card (`/.well-known/agent-card.json` + `GetExtendedAgentCard`) is now the spec §4.4.1 `AgentCard`: `supportedInterfaces` (`{url, protocolBinding, protocolVersion}`), `capabilities` object, `skills`, `provider`, `defaultInput/OutputModes` — not a flat method list. `protocolVersion` is per-interface (`"1.0"`); URLs host-relative pending a configurable origin | `crates/maidan-server/src/a2a_agent.rs` |

## v284.0.0 — A2A per-task push notification configs (compliance arc, part 3)

| Change | Where |
|--------|-------|
| A2A push configs are now per-task with a stable `configId` (spec model), not one-per-workspace. New `maidan_a2a_task_push_configs` table + `create`/`get`/`list`/`delete` store methods both backends; delivery fans out to all a task's configs | `migrations/{postgres/0049,sqlite/0048}_a2a_task_push_configs.sql`, `crates/maidan-store/src/{store.rs,postgres/a2a.rs,sqlite/a2a.rs,postgres/mod.rs,sqlite/mod.rs}` |
| `Create`/`Get`/`List`/`Delete` TaskPushNotificationConfig JSON-RPC ops (per-task, RBAC-checked via `ensure_task_workspace_access`); advertised in the Agent Card | `crates/maidan-a2a/src/protocol.rs`, `crates/maidan-server/src/a2a_agent.rs` |

## v283.0.0 — A2A `ListTasks` + `GetExtendedAgentCard` (compliance arc, part 2)

| Change | Where |
|--------|-------|
| A2A `ListTasks` op: workspace-scoped task list, `contextId`/`pageSize` filters, per-channel RBAC-filtered (drops tasks whose context thread the caller can't read); new `Store::list_a2a_tasks` both backends. Single-page (status filter/pagination deferred) | `crates/maidan-a2a/src/protocol.rs`, `crates/maidan-store/src/{postgres,sqlite}/a2a.rs`, `crates/maidan-server/src/a2a_agent.rs` |
| A2A `GetExtendedAgentCard` op (shared `agent_card_payload()`); both ops advertised in the Agent Card | `crates/maidan-server/src/a2a_agent.rs` |

## v282.0.0 — A2A v1.0 method names (compliance arc, part 1)

| Change | Where |
|--------|-------|
| A2A JSON-RPC method strings canonicalized to the A2A v1.0 spec (§5.3 Method Mapping): `tasks/cancel`→`CancelTask`, `tasks/pushNotificationConfig/{set,get}`→`{Create,Get}TaskPushNotificationConfig`; dropped the non-spec `tasks/resubscribe` alias. `SendMessage`/`SendStreamingMessage`/`GetTask`/`SubscribeToTask` + `TASK_STATE_*` were already spec-correct. First step of the full multi-transport + TCK A2A arc | `crates/maidan-a2a/src/protocol.rs`, `crates/maidan-server/src/a2a_agent.rs`, `docs/Integration.md` |

## v281.0.0 — Published benchmark methodology (launch-readiness P1)

| Change | Where |
|--------|-------|
| Post→observer realtime-propagation latency measurement (`post_to_observer_latency`): times producer-post → WebSocket-observer-receive, reading the event concurrently with the POST. Plus `docs/Benchmark.md` (published): named hardware/commit/backend, reproduction commands, honest caveats. Measured on Apple M3 Max / in-process SQLite: post→observer p50 0.71 ms/p99 1.00 ms; mixed throughput 1 586 ops/s (8 workers) / 666 ops/s (32, single-writer ceiling), 0 errors | `crates/maidan-server/tests/loadgen.rs`, `docs/Benchmark.md`, `book/src/SUMMARY.md`, `book/sync-docs.sh`, `README.md` |
| Loadgen SQLite target now uses the shipped 1-connection default (was 16 → the write-contention deadlock Cluster 277 fixed) | `crates/maidan-server/tests/loadgen.rs` |

## v280.0.0 — Framework integration recipes (launch-readiness P1)

| Change | Where |
|--------|-------|
| Copy-paste, live-verified LangChain / AutoGen / REST recipes: point a framework at Maidan's MCP Streamable HTTP endpoint and load all 78 tools (LangChain `MultiServerMCPClient`, AutoGen `StreamableHttpServerParams`), or use the `httpx` REST client. Guide carries the endpoint/token contract, the `mcp>=1.9,<2` pin, and AutoGen's every-param-needs-a-`type` rule; verified against a live Maidan | `examples/`, `docs/Framework Integrations.md`, `book/src/SUMMARY.md`, `book/sync-docs.sh`, `README.md` |
| Every MCP catalog tool parameter now declares a JSON-Schema `type` (`set_thread_result.result` was untyped → AutoGen's strict Pydantic converter rejected it) | `crates/maidan-mcp/src/tools/catalog.rs` |

## v279.0.0 — `maidan init` production-safe bootstrap (launch-readiness P0)

| Change | Where |
|--------|-------|
| `maidan init` CLI: one-time first-admin bootstrap (workspace + admin member + all-capabilities token, printed once) through the store; runs migrations, refuses on an already-initialized database. Removes the bootstrap chicken-and-egg so production needs no `AUTH_DISABLED` or public bootstrap routes. New `capability::all()`; documented in Production.md; integration-tested | `crates/maidan-cli/src/main.rs`, `crates/maidan-auth/src/capability.rs`, `crates/maidan-cli/tests/init.rs`, `docs/Production.md` |

## v278.0.0 — One-command quickstart (launch-readiness P0)

| Change | Where |
|--------|-------|
| `docker compose -f compose.quickstart.yaml up -d --build` + `scripts/quickstart-two-agents.sh` = clean machine → two agents collaborating, no Rust toolchain. Pinned, SHA-verified `v277.0.0` release binary on `ubuntu:24.04`, non-root, SQLite + localfs + loopback + dev auth-disabled ack; demo posts/reads/replies to show durable shared state. Built + run end-to-end locally; CI guards file validity (`compose config` + `bash -n`) | `docker/Dockerfile.quickstart`, `compose.quickstart.yaml`, `scripts/quickstart-two-agents.sh`, `README.md`, `.github/workflows/ci.yml` |

## v277.0.0 — SQLite write-contention fix (launch-readiness P0)

| Change | Where |
|--------|-------|
| SQLite no longer deadlocks on concurrent writes ("database is locked"). Root cause: single-writer SQLite + sqlx deferred `pool.begin()` on a multi-connection pool → read-then-write upgrade deadlock (`busy_timeout` can't resolve it; a harness showed ~90% of contended writes failing at 8 connections). Fix: SQLite backend defaults to 1 connection (`DEFAULT_SQLITE_MAX_CONNECTIONS`, override `MAIDAN_DB_MAX_CONNECTIONS`); Postgres unaffected. Regression guard `sqlite_write_contention` | `maidan-store/src/lib.rs`, `maidan-server/src/main.rs`, `maidan-store/tests/sqlite_write_contention.rs` |

## v276.0.0 — Runtime version truthfulness (launch-readiness P0)

| Change | Where |
|--------|-------|
| `/health` (and the binary/image) now report the release tag instead of `0.0.0`. The `MAIDAN_VERSION` override already existed; the release pipeline now sets it on every build path — native binaries (`release.yml`), the aarch64 cross build (`Cross.toml` passthrough), and the server image (`Dockerfile` `ARG`/`ENV` + `build-args`). New `build.rs` `rerun-if-env-changed=MAIDAN_VERSION` prevents a warm cache from shipping a stale version. Cargo `version` stays `0.0.0` (`publish = false`) | `crates/maidan-server/{build.rs,Dockerfile}`, `Cross.toml`, `.github/workflows/release.yml` |

## v275.0.0 — The pitch (docs)

| Change | Where |
|--------|-------|
| Final tagline + pitch — *"The operating layer for teams of AI agents"* + *"Run your agents as one coordinated team that works from a shared, durable memory and spends only the tokens it needs"* — threaded through README/Integration/Architecture/OpenAPI-description. Body: the gap (glue pile + token waste + lost work) → the combination that closes it (coordinate + durable record + targeted context + scoped access) → outcome (better work, fewer tokens). Access control first-class; em-dashes/AI-voice tells removed from the pitch + README first screen. Supersedes the 274 hook | `README.md`, `docs/{Integration,Architecture}.md`, `openapi/mod.rs` |

## v274.0.0 — Launch positioning + review reconciliation (docs)

| Change | Where |
|--------|-------|
| New problem-first pitch off "Slack for agents" ("AI agents are brilliant and forgetful…") threaded through README/Integration/Architecture/OpenAPI-description; fixed the broken `AUTH_DISABLED` quickstart command (fails closed since 157); relabeled A2A as an experimental subset + "what Maidan is not"; refreshed Architecture baseline `v179`→`v273`; folded a verified external launch-readiness review into a new Open Work "Public-launch readiness" backlog (version-truthfulness, SQLite first-write lock, quickstart, `maidan init`, LangChain/AutoGen recipes+CI, benchmark, A2A v1.0 compliance, GitHub metadata) | `README.md`, `docs/{Integration,Architecture,Open Work}.md`, `openapi/mod.rs` |

## v273.0.0 — Strategy-pack reconciliation (docs/governance)

| Change | Where |
|--------|-------|
| Committed a separate agent's 8-doc strategy pack (Handoff/Pre-Public Hardening/Path to Impressive/Expansion Bets/Launch/Promotion/Protocols/Providers) after a per-doc accuracy review; restored Open Work.md/Roadmap.md as the single canonical backlog (reverted the "Handoff.md is the backlog" redirect in CLAUDE.md/README) and folded the pack's genuinely-open items into a new "Post-272 forward work" section (MCP `2026-07-28` upgrade, durable mail retry queue, MCP example pack, SDKs, Slack/Git projectors, cleanup nits, launch). Fixed 2 mdbook linkcheck breakers, reframed the unregistered `maidan.world` domain as planned (not live), + same-day staleness. Docs-only | `docs/{Open Work,Roadmap,Handoff,README,Providers,Expansion Bets, …}.md`, `CLAUDE.md` |

## v272.0.0 — Optional deferrals: search replica-reads metric (sweep closes)

| Change | Where |
|--------|-------|
| `maidan_search_replica_reads_total{outcome}` — the search-side twin of `maidan_replica_reads_total`. `PostgresSearch` gets a metrics-agnostic `SearchReadMetrics` incremented in `read_pool` (replica-only); `main.rs` captures the handle onto `AppState`, `metrics.rs` delta-syncs it. No separate lag gauge (store's poller covers the shared replica). **Closes the optional-deferrals sweep (267–272) + the LSN read-replica program end-to-end** | `maidan-search/src/postgres.rs`, `maidan-server/src/{state.rs,main.rs,metrics.rs}`, `docs/Production.md` |

## v271.0.0 — Optional deferrals: search token-aware read routing

| Change | Where |
|--------|-------|
| `PostgresSearch` routes reads to a replica once caught up to the request's `Maidan-Consistency-Token` — own reader pool + 200 ms replay poller + `read_pool()`; single-sourced via new `maidan_store::postgres::replica_route` (reads the shared `READ_CONSISTENCY` task-local). Lexical + semantic reads route (semantic resolve + query share one pool); embedding writes/DDL/reindex stay primary. Wired at boot on `MAIDAN_DB_REPLICA_URL`; validated vs real streaming replication (`#[ignore]`d `replica_routing`) | `maidan-search/src/postgres.rs`, `maidan-store/src/postgres/mod.rs`, `maidan-server/src/main.rs`, `docs/Production.md` |

## v270.0.0 — Optional deferrals: workspace import (REST)

| Change | Where |
|--------|-------|
| `POST /workspaces/import` (`token:admin`) — write-side inverse of the 187 export over the 269 store. Body = the export bundle (`WorkspaceExport` now `Deserialize`). `?mode=new` (default) remaps every id → fresh workspace; `?mode=restore` preserves ids (409 if it exists, unless `&force=true` erases first). Pure `import::remap` (fresh ids + full FK rewrite) + `import::flatten` unit-tested; route proven e2e | `maidan-server/src/routes/workspace.rs`, `src/import.rs`, `src/export.rs`, `src/dto.rs`, `src/app.rs`, `openapi/`, `contracts/http-capability-map.json` |

## v269.0.0 — Optional deferrals: workspace import (store)

| Change | Where |
|--------|-------|
| `WorkspaceImport` bundle type (deserializable mirror of the 187 `WorkspaceExport`) + `Store::import_workspace` — one transaction, all-or-nothing, full-column inserts preserving explicit ids/state/timestamps (an exported bundle round-trips faithfully). Both backends (pg JSONB / sqlite JSON TEXT). Zero-blast-radius store foundation; mode flag + `token:admin` REST route + 409 guard land in 270 | `maidan-types/src/models.rs`, `maidan-store/src/store.rs`, `maidan-store/src/{postgres,sqlite}/import.rs` |

## v268.0.0 — Optional deferrals: MCP email-address tools

| Change | Where |
|--------|-------|
| `set_member_email` / `get_member_email` / `delete_member_email` MCP tools (`workspace:read`, member-scoped) — MCP twins of the 250 REST over the 248 store; `set` light `@` check → `InvalidParams`, `get` → address or `null`, `delete` → `{deleted}`. No new store logic | `maidan-mcp/src/tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v267.0.0 — Optional deferrals: A2A egress content → parts

| Change | Where |
|--------|-------|
| `message_parts_from_content` (egress inverse of `message_content`) + the A2A agent renders its outbound message from the stored message's canonical content (per-block text projection, mirroring `derive_body`) instead of echoing the request. Closes the federation egress deferral | `maidan-a2a/src/protocol.rs`, `maidan-server/src/a2a_agent.rs` |

## v266.0.0 — Program D (read-replica arc closer): lag gauge + docs

| Change | Where |
|--------|-------|
| `maidan_replica_lag_bytes` gauge (poller samples the primary write LSN too → `current − replay`) + a Production.md "Read replicas" section (config, `Maidan-Consistency-Token` contract, routing policy, metrics, test harness). **Closes the LSN read-replica arc (261–266) and Program D** | `maidan-store/src/postgres/mod.rs`, `maidan-server/src/metrics.rs`, `docs/Production.md` |

## v265.0.0 — Program D (read-replica arc): remaining read families + routing metric

| Change | Where |
|--------|-------|
| 28 more content/collaboration read delegations routed to `read_pool()` (skills/results/notifications/follows/emails/last-seen/channel-members/dm/group-dm/transitions/queue-depth/schedules/assigned/deps/edits/mentions/inbox/votes/reactions/usage), completing the member-facing read surface. Auth + control-plane/config reads deliberately stay on the primary. `maidan_replica_reads_total{outcome}` via a store-side `ReadRoutingMetrics`. Validated vs real replication | `maidan-store/src/postgres/mod.rs`, `maidan-server/src/{state,main,metrics}.rs` |

## v264.0.0 — Program D (read-replica arc): token ingestion + read routing

| Change | Where |
|--------|-------|
| `READ_CONSISTENCY` task-local + `with_read_consistency` (GET/HEAD-only scope) + `read_pool()`/pure `route_decision` + a background replay-LSN poller (cached in an atomic) + entity-read delegations routed to the replica once it has replayed past the client's token (else primary). Mutation/background reads stay on the primary. Validated vs real streaming replication; inert without a replica | `maidan-store/src/postgres/mod.rs`, `maidan-server/src/consistency.rs` |

## v263.0.0 — Program D (read-replica arc): consistency token on writes

| Change | Where |
|--------|-------|
| `Store::write_lsn()` (Postgres `pg_current_wal_lsn()`, SQLite `None`) + `AppState.read_replica_enabled` + `consistency::middleware` stamping `Maidan-Consistency-Token: <lsn>` on successful mutations when a replica is configured (captured after the handler — safely over-approximating; gated on the replica so no-replica deploys pay nothing). The write half of the causality contract — 264 routes on it | `store.rs`, `postgres/mod.rs`, `sqlite/mod.rs`, `state.rs`, `main.rs`, `consistency.rs`, `app.rs` |

## v262.0.0 — Program D (read-replica arc): reader-pool split (inert)

| Change | Where |
|--------|-------|
| `PostgresStore { pool, reader }` + `with_replica_reader` (`new` defaults reader=primary, no ripple to ~62 call sites); `MAIDAN_DB_REPLICA_URL` config + boot wiring (connects a real reader pool, fail-fast on a bad URL, same connection setup as primary). Reads still on the primary — the token-aware selector is a later cluster. Unset → zero behaviour change | `maidan-store/src/postgres/mod.rs`, `maidan-server/src/config.rs`, `main.rs` |

## v261.0.0 — Program D (read-replica arc): LSN primitives + replication harness

| Change | Where |
|--------|-------|
| `Lsn` causality-token type (`u64`-backed, `pg_lsn` parse/display, numeric `Ord`) + CI unit tests; store `current_wal_lsn`/`replica_replay_lsn`/`replica_caught_up`; `scripts/replica-harness.sh` (local pgvector primary + streaming standby); an `#[ignore]`d test validating the helpers against real replication. Validate-first keystone for LSN read-replica routing — inert (no read routed yet) | `maidan-types/src/lsn.rs`, `maidan-store/src/postgres/replication.rs`, `scripts/replica-harness.sh`, `maidan-store/tests/replication.rs` |

## v260.0.0 — Program D: backup / restore + DR runbook

| Change | Where |
|--------|-------|
| `scripts/backup.sh` (`pg_dump -Fc` + tar of the localfs artifact root + manifest) + `scripts/restore.sh` (`pg_restore`, refuses a non-empty target without `--force`) + a "Backup & disaster recovery" runbook (coverage, out-of-band secrets, S3-is-durable, RPO/RTO, recovery steps). Operator tools like `loadgen`/`chaos` | `scripts/backup.sh`, `scripts/restore.sh`, `docs/Production.md` |

## v259.0.0 — Program D: chaos / fault-injection harness

| Change | Where |
|--------|-------|
| An `#[ignore]`d chaos soak that publishes under load while killing the `LISTEN` backend (`pg_terminate_backend`), asserting no published event is lost — validates the Cluster-258 floor end-to-end (measured: 40/40 delivered across 5 kills). Pure `fault_due` cadence helper unit-tested in CI; the soak is a manual tool like `loadgen` (Docker + timing-sensitive). `scripts/chaos.sh` runner | `crates/maidan-bus/tests/chaos.rs`, `scripts/chaos.sh` |

## v258.0.0 — Program D: event-bus self-healing NOTIFY floor

| Change | Where |
|--------|-------|
| The PG `LISTEN`/`NOTIFY` bus tracks a high-water `log_id` and back-fills the missed range from the log on a gap (pointer id > `high_water+1`) or reconnect (drain to head) — the optimistic local broadcast no longer silently drops events appended during a `LISTEN` disconnect. Always hydrates the pointer's own id (no skip on `<= high_water`, so a concurrent late lower id isn't lost); batched, best-effort. `list_after_global`/`max_event_id`; `Backfilled` stat + `{result="backfilled"}` metric; `backfill()` heal hook | `maidan-bus/src/postgres.rs`, `maidan-store/src/postgres/events.rs`, `maidan-bus/src/hydrate_stats.rs`, `maidan-server/src/metrics.rs` |

## v257.0.0 — Program C (Arc I): delivery-mode MCP tools

| Change | Where |
|--------|-------|
| `set_delivery_mode` / `get_delivery_mode` MCP tools (`workspace:read`, member-scoped, no gate arm) — the twins of the 256 REST; `set` parses snake_case `immediate`/`digest` → `InvalidParams` on unknown, both return `{mode}`. Closes the core of Arc I (digest reachable over REST + MCP) | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v256.0.0 — Program C (Arc I): delivery-mode REST

| Change | Where |
|--------|-------|
| `PUT`/`GET /members/:id/delivery-mode` — set / read a member's email delivery mode (`immediate` or `digest`; `immediate` default), `workspace:read` + self-only. Request DTO wraps `EmailDeliveryMode` so an unknown mode is a `400`. Full new-route preflight | `routes/member.rs`, `dto.rs`, `app.rs`, `openapi/*`, `contracts/http-capability-map.json` |

## v255.0.0 — Program C (Arc I): digest sweeper + router honors digest mode

| Change | Where |
|--------|-------|
| Router skips the immediate email for a `Digest`-mode member (metered `skipped_digest`) | `notification_router.rs` |
| Opt-in digest sweeper (`MAIDAN_DIGEST_TICK_SECS`): drains `members_due_for_digest`, emails an unread-count rollup, advances `set_last_digest_at` on success (at-least-once, self-healing); no-op without a transport; not single-flighted (low-harm duplicate — run on one replica for exactly-once) | `digest.rs`, `main.rs`, `lib.rs`, `metrics.rs` |

## v254.0.0 — Program C (Arc I): email digest data model (store foundation)

| Change | Where |
|--------|-------|
| `EmailDeliveryMode` (`Immediate` default / `Digest`) + `DigestDue` | `maidan-types/src/models.rs` |
| `maidan_member_delivery_prefs` + `maidan_member_digest_state` tables (pg 0048 / sqlite 0047) + store `set/get_delivery_mode` (default `Immediate`), `set_last_digest_at` (watermark), `members_due_for_digest` (digest-mode members w/ address + unread-since-last-digest, address inline), both backends. The alternative-mode digest data model (immediate OR digest, not both). **Foundation** — unwired | `migrations/*`, `store/*/email_digest.rs` |

## v253.0.0 — Program C (Arc I): presence-aware email routing

| Change | Where |
|--------|-------|
| WS `/ws/subscribe` touches `last_seen` on presence registration (best-effort, spawned — never blocks the connect) | `ws.rs` |
| `deliver_notification_email` skips the send when the recipient was seen within `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS` (opt-in; unset/0 = send as before); `maidan_email_delivered_total{outcome="skipped_present"}`; fail-open on a read error. Wires the Cluster-252 store end-to-end | `notification_router.rs`, `metrics.rs` |

## v252.0.0 — Program C (Arc I): durable member last-seen (store foundation)

| Change | Where |
|--------|-------|
| `maidan_member_last_seen` table (pg 0047 / sqlite 0046; `member_id` PK, `last_seen_at`) + store `touch` (upsert `now()`) / `get` → `Option<DateTime<Utc>>`, both backends. The durable presence signal for presence-aware email routing (Cluster 253) — presence is in-memory only today. A separate table (not a member column) to avoid the row ripple; no model type. **Foundation** — unwired | `migrations/*`, `store/*/member_last_seen.rs` |

## v251.0.0 — Program C (Arc I): /ui notification center

| Change | Where |
|--------|-------|
| A "Notifications" tab in the `/ui` (list + unread badge + mark-read/read-all + unread-only filter) over four new `/ui/api/members/:id/notifications*` routes reusing the Cluster-239 handlers under the session middleware. `sessionMemberId` = self; no capability-map/OpenAPI churn (`/ui/api` curated subset); `ui_js_contract` green | `app.rs`, `static/index.html` |

## v250.0.0 — Program C (Arc I): member delivery-email REST

| Change | Where |
|--------|-------|
| `PUT`/`GET`/`DELETE /members/:id/email` — set (opt-in) / read (`404` unset) / clear (opt-out), `workspace:read` + self-only. Makes email opt-in usable over HTTP; light `@` check at the edge, full validation at the transport | `routes/member.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/http-capability-map.json` |

## v249.0.0 — Program C (Arc I): email delivery wired into the router

| Change | Where |
|--------|-------|
| The notification router now delivers a per-recipient notification by email to members with an address (Cluster 248), when an SMTP transport is configured (247). `AppState.mail` + `attach_mail`, built from `SmtpConfig::from_env` in `main.rs`; spawned best-effort (never blocks routing), `maidan_email_delivered_total{outcome}` metric. Presence of an address = opt-in | `state.rs`, `main.rs`, `notification_router.rs`, `metrics.rs` |

## v248.0.0 — Program C (Arc I): member delivery-email store

| Change | Where |
|--------|-------|
| `maidan_member_emails` table (pg 0046 / sqlite 0045; `member_id` PK, `email`, one per member) + `MemberEmail` + store `set`/`get`/`delete`, both backends. Where a member's email notifications go — the recipient-address prerequisite for the SMTP transport. A separate table (not a member column) to avoid the row ripple. **Foundation** — no delivery wiring yet | `migrations/*`, `models.rs`, `store/*/member_emails.rs` |

## v247.0.0 — Program C (Arc I): email/SMTP transport foundation

| Change | Where |
|--------|-------|
| `MailTransport` trait + `lettre`-backed `SmtpTransport` + `SmtpConfig::from_env` (`MAIDAN_SMTP_*`) — the first off-platform delivery transport, config-gated (no config → no mailer → nothing sent) and unwired. `lettre` on the existing rustls+tokio stack; `cargo deny` green with `0BSD` allowed | `mail.rs`, `lib.rs`, `Cargo.toml`, `deny.toml` |

## v246.0.0 — Program C (Arc H complete): follows MCP tools

| Change | Where |
|--------|-------|
| MCP `follow_channel` / `unfollow_channel` / `list_channel_follows` + the thread triple — the twins of Cluster 245's REST, over the shared store (`workspace:read`, member-scoped; `follow_*` gate on target access). **Completes Arc H** (mute 241–243, follows 244–246) over REST + MCP | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v245.0.0 — Program C (Arc H): follows-aware router + follow REST

| Change | Where |
|--------|-------|
| The router fans `MessagePosted` → channel + thread followers (minus the author, mute-aware) via a shared `notify` helper — following delivers new activity to the inbox | `notification_router.rs` |
| `POST`/`GET /members/:id/channel-follows` + `DELETE …/:cid` and the thread triple — follow/unfollow/list, `workspace:read` + self-only, follow gated on target access | `routes/member.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/http-capability-map.json` |

## v244.0.0 — Program C (Arc H): follows/subscription foundation

| Change | Where |
|--------|-------|
| `maidan_channel_follows` + `maidan_thread_follows` tables (pg 0045 / sqlite 0044; PK `(member, target)`, reverse index; presence = following) + `ChannelFollow`/`ThreadFollow` + store follow/unfollow/list/`*_followers` (the router's fan-out set), both backends. A member follows a channel or thread to be notified of activity there. **Zero-blast-radius foundation** — no router change/routes yet | `migrations/*`, `models.rs`, `store/*/follows.rs` |

## v243.0.0 — Program C (Arc H): mute-preference MCP tools

| Change | Where |
|--------|-------|
| MCP `set_notification_pref` (upsert a per-`EventKind` mute; `kind` snake_case string) / `list_notification_prefs` — the twins of Cluster 242's REST, over the shared store (`workspace:read`, member-scoped). The mute half of Arc H is now complete over REST + MCP | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v242.0.0 — Program C (Arc H): mute-aware router + preferences REST

| Change | Where |
|--------|-------|
| The notification router skips a muted `(member, kind)` (`route_event` consults `is_notification_muted`; `maidan_notifications_suppressed_total{reason}` metric) | `notification_router.rs`, `metrics.rs` |
| `PUT`/`GET /members/:id/notification-prefs` — set (upsert) / list a member's mutes; `workspace:read`, self-only for sessions (bearer act-as-any) | `routes/member.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/http-capability-map.json` |

## v241.0.0 — Program C (Arc H): notification mute-preferences foundation

| Change | Where |
|--------|-------|
| `maidan_notification_prefs` table (pg 0044 / sqlite 0043; PK `(member_id, kind)`, `muted` flag; one row per member × `EventKind`, absent = notify) + `NotificationPref` + store `set_notification_pref` (upsert) / `list_notification_prefs` / `is_notification_muted` (router query), both backends. The routing brain the notification router will consult. **Zero-blast-radius foundation** — no router change/routes yet; opens Arc H | `migrations/*`, `models.rs`, `store/*/notification_prefs.rs` |

## v240.0.0 — Program C (Arc G complete): MCP inbox tools + `wait_for_notification`

| Change | Where |
|--------|-------|
| MCP `list_notifications` / `get_unread_count` / `mark_notification_read` (`workspace:read`) — the twins of Cluster 239's REST, over the shared store | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| MCP `wait_for_notification` — block on the member's next notification-worthy event (the general form of `wait_for_mention`; shared `wait_for_member_event` helper). **Closes Arc G** (ledger 237 → router 238 → REST 239 → MCP 240) | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v239.0.0 — Program C (Arc G): REST unified inbox

| Change | Where |
|--------|-------|
| `GET /members/:id/notifications` (list; `unread_only`, `limit`) + `GET …/unread-count` + `POST …/:nid/read` (returns new count) + `POST …/read-all` (`{cleared}`) — all `workspace:read`, **self-only** for sessions (bearer act-as-any). The read side of the Cluster-237 ledger | `routes/member.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/http-capability-map.json` |
| `mark_notification_read` recipient-scoped in the store (`(member_id, id)`) — safe-by-construction; `404` for a foreign/unknown id | `store/*/notifications.rs`, `store.rs` |

## v238.0.0 — Program C (Arc G): notification router

| Change | Where |
|--------|-------|
| `NotificationRouter` — an always-on, reconnecting event-bus consumer (spawned in `main.rs`, drained on shutdown) that resolves an event to the members it concerns and writes per-recipient rows. Routes `MentionRecorded` → the mentioned member (channel resolved from the thread) | `notification_router.rs`, `lib.rs`, `main.rs` |
| `create_notification_if_absent` (`ON CONFLICT DO NOTHING`) + `UNIQUE(member_id, source_log_id)` index (pg 0043 / sqlite 0042) — cross-replica/replay-idempotent writes; `maidan_notifications_created_total{kind}` metric | `store/*/notifications.rs`, `migrations/*`, `metrics.rs` |

## v237.0.0 — Program C (Arc G): per-recipient notification ledger

| Change | Where |
|--------|-------|
| `maidan_notifications` table (pg 0042 / sqlite 0041; one row per recipient × source event — `member_id`, `kind`=`EventKind`, `source_log_id` (no FK), denormalized `channel/thread/message/actor`, `read_at` NULL=unread) + `Notification`/`NewNotification` + store CRUD (create / list / mark-read / mark-all / unread-count), both backends. The per-recipient layer a mention's shared row + single cursor can't express. **Zero-blast-radius foundation** — no router/routes yet; opens Program C | `migrations/*`, `models.rs`, `store/*/notifications.rs` |

## v236.0.0 — Program B (Arc F complete, Program B complete): structured-results MCP + `wait_for_result`

| Change | Where |
|--------|-------|
| MCP `set_thread_result` (`thread:transition`) / `get_thread_result` (`workspace:read`) — the twins of Cluster 235's REST, over the shared store; `set` publishes `ThreadResultSet` | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| MCP `wait_for_result` (`workspace:read`) — block on a thread's `ThreadResultSet`, return the result payload (or `null` on timeout); the coordination wait, the `wait_for_ready` analogue | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| MCP `get_dependency_results` (`workspace:read`) — a parent aggregates its dependencies' outputs as `[{thread_id, result}]` (`null` for pending), RBAC-filtered. **Closes Program B** | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v235.0.0 — Program B (Arc F): structured-results REST + `ThreadResultSet` event

| Change | Where |
|--------|-------|
| `PUT /threads/:id/result` (`thread:transition`) upserts a task's structured JSON result + `GET /threads/:id/result` (`workspace:read`) reads it back (`404` until produced), both under DM-participant-aware thread RBAC. Wires the Cluster-234 store foundation | `routes/thread.rs`, `dto.rs`, `app.rs`, `openapi/*`, `contracts/http-capability-map.json` |
| `ThreadResultSet` event on set — a "go fetch" pointer (`{workspace, channel, thread, produced_by}`, no payload inline), observable on WS + MCP-SSE like `ThreadReady`; locally-derived → **non-federatable** (allowlist excludes it with `ArtifactUpserted` + `ThreadReady`) | `maidan-types/src/events.rs`, `federation.rs`, `contracts/event-kinds.json` |

## v234.0.0 — Program B (Arc F): structured-results foundation

| Change | Where |
|--------|-------|
| `maidan_thread_results` table (pg 0041 / sqlite 0040; `thread_id` PK, `result` JSONB/TEXT, `produced_by`, `produced_at`) + `ThreadResult` + `Store::set_thread_result` (upsert) / `get_thread_result`, both backends. A task's structured output; a requester or parent task reads it back. **Zero-blast-radius foundation** — no worker/routes yet | `migrations/*`, `models.rs`, `store/*/thread_results.rs` |

## v233.0.0 — Program B (Arc E complete): capability-registry MCP tools

| Change | Where |
|--------|-------|
| MCP `add_member_skill` / `list_member_skills` (`workspace:write`/`read`) + `add_thread_required_skill` / `list_thread_required_skills` (`thread:transition` + channel access / `workspace:read`) over the shared store — the MCP twin of Cluster 232's REST. **Arc E complete**: skill routing surfaced over REST + MCP, enforced in `claim_next` | `tools/skill.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v232.0.0 — Program B (Arc E): capability-registry REST

| Change | Where |
|--------|-------|
| Member-skill CRUD (`POST`/`GET /members/:id/skills`, `DELETE …/:skill`; `workspace:write`/`workspace:read`) + thread required-skill CRUD (`POST`/`GET /threads/:id/required-skills`, `DELETE …/:skill`; `thread:transition` + thread access / `workspace:read`). Drives the Cluster-231 skill routing from outside the store. Full new-route preflight (6 routes) | `routes/skills.rs`, `app.rs`, `openapi/*`, `contracts/http-capability-map.json` |

## v231.0.0 — Program B (Arc E): skill-aware claim

| Change | Where |
|--------|-------|
| `maidan_thread_required_skills` table (pg 0040 / sqlite 0039) + `ThreadRequiredSkill` + store CRUD, **and** `claim_next`/`claim_next_with_event` skip a task whose required skills the claimer lacks (a `NOT EXISTS` clause beside the readiness one; 4 SQL sites, both backends). Set containment — no-requirement tasks claimable by anyone. The existing claim route + `claim_next_thread` MCP become skill-routing for free | `migrations/*`, `models.rs`, `store/*/thread_skills.rs`, `store/*/threads.rs` |

## v230.0.0 — Program B (Arc E): capability-registry foundation

| Change | Where |
|--------|-------|
| `maidan_member_skills` table (pg 0039 / sqlite 0038) + `MemberSkill` + 3 store methods (add idempotent / remove conditional / list), both backends. Free-form skill tags an agent declares; skill routing (231+) matches a task's required skills by set containment. **Zero-blast-radius foundation** — no worker/routes yet (159/217/226 pattern) | `migrations/*`, `models.rs`, `store/*/member_skills.rs` |

## v229.0.0 — Program B: task-schedule MCP tools

| Change | Where |
|--------|-------|
| MCP `create_task_schedule` (`workspace:write`, channel-gated) + `list_task_schedules` (`workspace:read`, channel-filtered) over the shared store — so an MCP-only agent schedules its own recurring/one-shot work. The MCP twin of the Cluster 228 REST endpoints; completes the scheduler subsystem (store 226 → worker 227 → REST 228 → MCP 229) | `tools/schedule.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v228.0.0 — Program B: task-schedule REST management API

| Change | Where |
|--------|-------|
| `POST/GET /workspaces/:wid/task-schedules` + `PUT/DELETE /task-schedules/:id` — create/list/pause-resume/delete schedules. Writes gated on `workspace:write` + target-channel access; list on `workspace:read`. `Store::set_task_schedule_active`. Full new-route preflight | `routes/task_schedule.rs`, `app.rs`, `store/*/task_schedules.rs`, `openapi/*`, `contracts/http-capability-map.json` |

## v227.0.0 — Program B: scheduler sweeper worker

| Change | Where |
|--------|-------|
| Background scheduler sweeper (opt-in `MAIDAN_SCHEDULER_TICK_SECS`): each tick fires due schedules — `Store::claim_next_due_schedule` atomically claims + advances (`FOR UPDATE SKIP LOCKED` on pg, so replicas don't double-fire; recurring re-arms to `now + interval`, one-shot deactivates), then creates the task thread. At-most-once on crash (claim commits first). `maidan_task_schedules_fired_total{outcome}` metric. Off by default | `scheduler.rs`, `main.rs`, `store/*/task_schedules.rs`, `metrics.rs` |

## v226.0.0 — Program B: scheduled/recurring task foundation

| Change | Where |
|--------|-------|
| `maidan_task_schedules` table (pg 0038 / sqlite 0037) + `TaskSchedule`/`NewTaskSchedule` + `TaskScheduleId` + 5 store methods (create/get/list/delete + `due_task_schedules` scan), both backends. A schedule materializes a task thread when due (`interval_secs` NULL = one-shot, positive = recurring). **Zero-blast-radius foundation** — no worker/routes yet (159/217 pattern) | `migrations/*`, `models.rs`, `ids.rs`, `store/*/task_schedules.rs` |

## v225.0.0 — Program B: `get_queue_depth` MCP tool

| Change | Where |
|--------|-------|
| MCP `get_queue_depth` (`workspace:read`, channel-gated): `{channel_id}` → `{open, ready, assigned, blocked}` over the shared `Store::channel_queue_depth` — the MCP twin of Cluster 224's REST endpoint, so an MCP-only orchestrator can read queue depth | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v224.0.0 — Program B: channel task-queue depth

| Change | Where |
|--------|-------|
| `GET /channels/:cid/queue-depth` (`workspace:read` + channel access) → `{ open, ready, assigned, blocked }`: a point-in-time partition of a channel's open task threads for scaling decisions. `ready` = the `claim_next` predicate; one aggregate query per backend (`Store::channel_queue_depth`); on-demand DB aggregate, not a per-channel metric (Cluster 188 cardinality decision) | `models.rs`, `store/*/threads.rs`, `routes/channel.rs` |

## v223.0.0 — Program B: `wait_for_ready` MCP long-poll

| Change | Where |
|--------|-------|
| MCP `wait_for_ready` (`workspace:read`): blocks until a task becomes claimable (subscribes to `ThreadReady`), returning the ready thread or `null` on timeout (default 30 s, clamp 1 ms–300 s). Optional `channel_id` scope (access-checked pre-dispatch); else any accessible thread in the workspace, RBAC-filtered per event. The `wait_for_mention` analogue for the DAG; completes the DAG surface end-to-end | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v222.0.0 — Program B: reactive task readiness (`ThreadReady`)

| Change | Where |
|--------|-------|
| New `ThreadReady` event: a terminal thread transition that unblocks dependents publishes `ThreadReady { workspace_id, channel_id, thread_id, thread }` for each newly-ready task, so an agent can subscribe (`kinds=thread_ready`) instead of polling `dependencies_satisfied`. Backed by `Store::newly_ready_dependents` (both backends); emitted only on a non-terminal → terminal edge; best-effort; **non-federatable** (locally-derived signal) | `events.rs`, `store/*/thread_deps.rs`, `routes/thread.rs`, `federation.rs`, `contracts/event-kinds.json` |

## v221.0.0 — Program B: task-DAG transitive cycle prevention

| Change | Where |
|--------|-------|
| `add_thread_dependency` rejects any edge that would close a cycle (direct or transitive), not just self-loops — a recursive-CTE reachability check before insert, check + insert in one transaction, `InvalidInput` (REST `400` / MCP `InvalidParams`). Both backends; no schema/route/tool/contract change. The task-dependency DAG is now actually acyclic | `store/{sqlite,postgres}/thread_deps.rs` |

## v220.0.0 — Program B: task-dependency DAG MCP tools

| Change | Where |
|--------|-------|
| MCP `add_thread_dependency` (`thread:transition`; both-thread RBAC + same-workspace) + `list_thread_dependencies` (`workspace:read`; returns `{dependencies, ready}`). Full 5-place wiring (handlers, dispatch, capability, pre-dispatch gate, catalog, both `contracts/mcp-*.json`). Completes the DAG read/write surface over REST + MCP | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## v219.0.0 — Program B: task-dependency DAG management API (REST)

| Change | Where |
|--------|-------|
| REST DAG management: `POST/GET /threads/:id/dependencies` (add; list + `ready`), `DELETE /threads/:id/dependencies/:dep_id`, `GET /threads/:id/dependents`. RBAC on both edge threads + same-workspace; `thread:transition` mutations / `workspace:read` reads. Full new-route preflight (OpenAPI paths+schemas, http-capability-map, matrix) | `routes/thread.rs`, `app.rs`, `dto.rs`, `openapi/*` |

## v218.0.0 — Program B: readiness-aware `claim_next`

| Change | Where |
|--------|-------|
| `claim_next` / `claim_next_with_event` (both backends) skip tasks with a non-terminal dependency (a `NOT EXISTS` clause in the candidate subquery/CTE) — the "pull next task" primitive respects the DAG. Existing REST `claim-next` route + MCP `claim_next_thread` tool become dependency-aware with no new API | `store/*/threads.rs` |

## v217.0.0 — Program B: task-dependency DAG (store foundation)

| Change | Where |
|--------|-------|
| `maidan_thread_dependencies` edge table (both backends; pg 0037 / sqlite 0036) + `ThreadDependency` model + `ThreadState::is_terminal()` + store methods (add/remove/list-dependencies/list-dependents/dependencies-satisfied — readiness = all deps terminal). Zero-blast-radius foundation (no routes yet); reuses the thread-as-task model. Opens **Program B (agentic orchestration)** | migrations, `store/*/thread_deps.rs` |

## v216.0.0 — Security: RLS spike (deferred); Program A complete

| Change | Where |
|--------|-------|
| Row-Level Security assessed as defense-in-depth beneath app-layer RBAC → **deferred** (decision ADR: RLS design, blockers — shared pool/workspace-agnostic Store/SQLite-no-RLS/orchestrator model — and trigger conditions). App-layer RBAC stays authoritative. Concludes **Program A (202–216)** | `docs/Decisions.md` (`## Security`) |

## v215.0.0 — Security: federation ingest trust policy

| Change | Where |
|--------|-------|
| `EventKind::federatable()` allowlist (allowlist-by-default via exhaustive match; `ArtifactUpserted` excluded — blobs aren't federated) enforced on ingest (`403` for non-federatable, both push endpoint + pull worker); `MemberJoined` remap now re-scopes the nested `member.workspace_id` to local (no remote-id leak) | `maidan-types/src/events.rs`, `federation.rs` |

## v214.0.0 — Correctness: transactional outbox (references + artifacts; domain migration complete)

| Change | Where |
|--------|-------|
| `add_reference_with_event` (`ReferenceAdded`, scope-less) + `upsert_artifact_with_event(new, ref_workspace)` — upsert + Cluster-204 access ref + `ArtifactUpserted` in ONE tx (new `record_ref_in_tx`; preserves upsert→ref→event ordering, strengthens 204 isolation). Both upload routes use it. **Completes the domain-mutation outbox migration** — `publish()`'s only remaining caller is the federation relay | `store/*/{refs,artifacts}.rs`, `routes/{reference,artifact}.rs` |

## v213.0.0 — Correctness: transactional outbox (A2A ingest + member/workspace creation)

| Change | Where |
|--------|-------|
| A2A ingest post reuses `post_message_with_event(new, None)` (DM-post shape); `create_member_with_event` (`MemberJoined`) + `create_workspace_with_event` (`WorkspaceCreated`) — insert + event in one tx (no scope resolution; the created entity is the subject). Routes use them + `publish_stored`. `publish()` remains only for reference/artifact events (+ federation relay) | `a2a_agent.rs`, `store/*/{members,workspaces}.rs` |

## v212.0.0 — Correctness: transactional outbox (message edit + tombstone)

| Change | Where |
|--------|-------|
| `edit_message_with_event` (`MessageEdited`) + `tombstone_message_with_event` (`MessageTombstoned`) — mutation + event in one tx; shared `edit_in_tx` core (with 211's posted variant); tombstone keeps its `NotFound`-on-no-op guard. Routes use them + `publish_stored` → `message.rs` is now `publish()`-free. `publish()` remains only for A2A ingest + member/workspace/reference/artifact (+ federation relay) | `store/*/messages.rs`, `routes/message.rs` |

## v211.0.0 — Correctness: transactional outbox (regular message post)

| Change | Where |
|--------|-------|
| Regular `post_message` route branches — no-slash → `post_message_with_event` (atomic insert+event); slash → provisional insert, external dispatch, then `edit_message_with_posted_event` (edit + `MessagePosted` of the edited message in one tx, via new `message_edits::append_in_tx`). Closes the message-post hold-out; `publish()` retained for edit/tombstone/A2A/member/workspace/reference/artifact + federation relay | `store/*/{messages,message_edits}.rs`, `routes/message.rs` |

## v210.0.0 — Correctness: transactional outbox (DM / group-DM posts)

| Change | Where |
|--------|-------|
| `post_message_with_event(new, dm_conversation_id)` — message insert + `MessagePosted` in one tx (via `message_scope_in_tx`; `dm_conversation_id` Some for 1:1 / None for group). DM + group-DM post routes use it + `publish_stored`. The regular slash-editing post path is the last `publish()` holdout | `store/*/messages.rs`, `dm.rs`, `group_dm.rs` |

## v209.0.0 — Correctness: transactional outbox (thread assignments)

| Change | Where |
|--------|-------|
| `assign/unassign/claim/claim_next_thread_with_event` — assignee change + `ThreadAssignmentChanged` in one tx (reuses 208's `thread_scope_in_tx`; shared `append_assignment_event`); assign/unassign capture previous in-tx (fixes a read-then-write race), claim/claim_next conditional. Routes use them + `publish_stored`; `publish_assignment` helper removed. Completes the thread-scoped outbox batch | `store/*/threads.rs`, `routes/thread.rs` |

## v208.0.0 — Correctness: transactional outbox (thread transitions)

| Change | Where |
|--------|-------|
| `transition_thread_with_event` — FSM state change + `ThreadStateChanged` event in one tx, over a new `events::thread_scope_in_tx` resolver (thread-scoped twin of 206's message resolver); the FSM step is extracted into a shared `transition_in_tx` core so the non-event path is unchanged. Route uses it + `publish_stored`. Continues the 205–207 outbox migration | `store/*/{thread_transitions,events}.rs`, `routes/thread.rs` |

## v207.0.0 — Correctness: transactional outbox (pins + mentions)

| Change | Where |
|--------|-------|
| `pin_message_with_event` / `unpin_message_with_event` / `record_mention_with_event` — row + event in one tx over the shared `events::message_scope_in_tx` resolver (pins carry the channel; unpin emits `MessageUnpinned` only when a row was removed); routes use them + `publish_stored`. Continues the 205/206 outbox migration | `store/*/{pins,mentions,events}.rs`, `routes/{social,message}.rs` |

## v206.0.0 — Correctness: transactional outbox (votes + reactions)

| Change | Where |
|--------|-------|
| `cast_vote_with_event` / `add_reaction_with_event` / `remove_reaction_with_event` — row + event in one tx (shared `events::message_scope_in_tx` resolver; remove emits only when a row was removed); routes use them + `publish_stored`. Continues the 205 outbox migration | `store/*/{votes,reactions,events}.rs`, `routes/social.rs` |

## v205.0.0 — Correctness: transactional outbox (foundation)

| Change | Where |
|--------|-------|
| `events::append_in_tx(&mut tx, event)` (both backends) + `create_channel_with_event` / `create_thread_with_event` — insert the domain row **and** append its event (+ outbox) in one transaction (atomic dual-write); routes use them + `publish_stored` for the post-commit bus notify. First step of the multi-cluster transactional-outbox refactor (the 184 deferral); remaining mutations follow | `store/*/{events,channels,threads}.rs`, `routes/{mod,channel,thread}.rs` |

## v204.0.0 — Security: cross-tenant artifact isolation

| Change | Where |
|--------|-------|
| `maidan_artifact_refs` (workspace_id, sha256) link table — a ref is written on upload; `get_artifact*` requires a matching ref for the caller's workspace (404 if absent, no existence oracle). Closes cross-tenant blob reads over the deduped store; dedup preserved (two workspaces uploading the same bytes each get a ref). Migration backfills from the uploader's workspace | `migrations/*/…artifact_workspace_refs.sql`, `store/*/artifacts.rs`, `routes/artifact.rs` |

## v203.0.0 — Security: DM/group-DM participation (subscribe + metadata)

| Change | Where |
|--------|-------|
| Subscribe gate: `expand_event_filter` runs `ensure_thread_access` (DM-participant-aware) on the resolved `thread_id` — a non-participant can no longer tail a DM/group-DM via `dm_conversation_id` or `thread_id` (WS + MCP-SSE) | `dm.rs`, `ws.rs`, `mcp_stream.rs` |
| Metadata reads: `GET /dm/:id` + `/group-dms/:id` require participation for a session caller; `list` is self-only (session). Bearer = orchestrator (act-as-any), bypass unrestricted | `dm.rs`, `group_dm.rs` |

## v202.0.0 — Security: session-bound acting identity (anti-spoofing)

| Change | Where |
|--------|-------|
| `ensure_acting_member(auth, claimed)` — a **session** caller may only act as its own member; applied to every member-attributed write (post/DM/group-DM/edit/vote/react/pin/unpin/transition/assign/unassign/claim/claim-next/renew). Bearer = act-as-any (unchanged); bypass unrestricted. Closes a session-impersonation vuln | `routes/mod.rs` + all write handlers |

## v201.0.0 — Perf: workspace-sharded event fan-out

| Change | Where |
|--------|-------|
| `ShardedBroadcast` — a publish reaches only the event's workspace shard + a global shard (cross-workspace subscribers), not every subscriber; fan-out is O(relevant) not O(all). Used by `InMemoryBus` + `PostgresBus` local broadcast; shards created on subscribe, pruned on last-receiver-drop. Behavior unchanged (optimization under the existing `EventFilter`) | `crates/maidan-bus/src/sharded.rs` |

## v200.0.0 — Perf + security: filtered-ANN search (RBAC deny in the query)

| Change | Where |
|--------|-------|
| Search excludes the caller's inaccessible private channels **in the query** (`SearchFilters::deny_channels`; SQLite `NOT IN`, Postgres `<> ALL($n)`; lexical + semantic) so a full page of accessible hits is returned instead of a post-filtered short page — DMs stay with the authoritative thread-level post-filter | `maidan-search/src/{sqlite,postgres}.rs` |
| `maidan_auth::private_channel_deny_set` — the private, non-DM channels the caller isn't a member of; wired into REST `GET …/search` + MCP `search_messages` | `maidan-auth/src/access.rs`, `routes/search.rs`, `tools/search.rs` |

## v199.0.0 — Perf: concurrent workspace-context assembly

| Change | Where |
|--------|-------|
| `build_workspace_context` builds each page thread's context via a bounded `buffered` stream (`CONTEXT_THREAD_CONCURRENCY=8`) instead of a sequential loop — collapses `Σ per-thread` latency toward `ceil(N/8)×`, order + query-count + error semantics unchanged | `crates/maidan-server/src/thread_context.rs` |

## v198.0.0 — Perf: load / soak harness (Arc D opener)

| Change | Where |
|--------|-------|
| `scripts/loadgen.sh` + `#[ignore]`d `load_baseline` test — concurrent REST load (post/read/search), reports per-op latency percentiles + throughput; in-process (SQLite) or external (`MAIDAN_LOADGEN_URL`); env-tunable concurrency/iterations/soak-duration; pure nearest-rank percentile math unit-tested in CI | `crates/maidan-server/tests/loadgen.rs`, `scripts/loadgen.sh` |

## v197.0.0 — Agentic: tool-call transcripts (Arc C finale)

| Change | Where |
|--------|-------|
| `tool_transcript` — walks a thread's messages, pairs every `ToolUse` with its `ToolResult` by id (order-independent), returns a token-lean `ToolTranscript` (ordered calls + `orphan_results`, drops text/code/body); tombstoned messages skipped | `maidan-types/src/models.rs` |
| REST `GET /threads/:id/tool-transcript` + MCP `get_tool_transcript` (both `workspace:read`, thread-RBAC, `limit` 1..=500 default 200) | `routes/thread.rs`, `tools/thread.rs` + OpenAPI + contracts |

## v196.0.0 — Agentic: `wait_for_mention` (blocking long-poll)

| Change | Where |
|--------|-------|
| MCP `wait_for_mention` — subscribes to the event bus filtered to the member's `MentionRecorded` events and blocks until one arrives or `timeout_ms` lapses (default 30 s, clamp 1 ms–300 s); returns the mention or `null`. Live-only (drain existing with `get_inbox` first); RBAC-filtered by `can_access_thread`. Requires `workspace:read` | `crates/maidan-mcp/src/tools/member.rs` + `mod.rs` + `catalog.rs` + both `contracts/mcp-*.json` |

## v195.0.0 — Agentic: handoff notes on thread assignment

| Change | Where |
|--------|-------|
| `assign_thread` (REST `PUT /threads/:id/assignee` + MCP tool) accepts an optional `note`; it rides the `ThreadAssignmentChanged` event to the new assignee + subscribers in real time (event-only, not persisted). Note-less claim/unassign/`claim_next` unchanged | `events.rs` + `dto.rs` + `routes/thread.rs` + `tools/{thread,catalog}.rs` + `federation.rs` |

## v194.0.0 — Agentic: A2A ingest preserves parts as structured content

| Change | Where |
|--------|-------|
| A2A `POST /a2a/v1/rpc` ingest maps text parts to `ContentBlock::Text` (was `content: None`), so A2A messages carry the same structured content as REST/MCP (Cluster 173); `body` unchanged | `maidan-a2a/src/protocol.rs` + `a2a_agent.rs` |

## v193.0.0 — Agentic: the `roots/list` tool

| Change | Where |
|--------|-------|
| MCP `list_roots` — server→client `roots/list` over the streamable session; the third `request_client` verb's first organic caller | `crates/maidan-mcp/src/tools/roots.rs` |

## v192.0.0 — Agentic: claim leases + reclaim (dead-agent recovery)

| Change | Where |
|--------|-------|
| `claim_next_thread` lease-aware (`lease_secs`; expired lease = reclaimable, no reaper) + `renew_claim` heartbeat (holder-only); `assignment_expires_at` column; REST `POST /threads/:id/claim/renew` + MCP `renew_claim` | `*/threads.rs` + `routes/thread.rs` + `tools/thread.rs` |

## v191.0.0 — Agentic: MCP tools for the assignment read-side

| Change | Where |
|--------|-------|
| MCP `claim_next_thread` (channel-gated pre-dispatch) + `list_assigned_threads` (member-scoped, RBAC-filtered aggregate read) | `maidan-mcp/src/tools/thread.rs` + `mod.rs` + `catalog.rs` + contracts |

## v190.0.0 — Agentic: thread-assignment read-side (my-queue + claim-next)

| Change | Where |
|--------|-------|
| `GET /members/:id/assigned-threads` (my work queue, RBAC-filtered) + `POST /channels/:cid/threads/claim-next` (atomically claim oldest unassigned; Postgres `FOR UPDATE SKIP LOCKED`) | `maidan-store/src/*/threads.rs` + `routes/thread.rs` |

## v189.0.0 — SaaS ops: secret-rotation keyring

| Change | Where |
|--------|-------|
| Try-all-keys decrypt keyring — rotate `FEDERATION_ENCRYPTION_KEY` by moving old keys into `FEDERATION_DECRYPT_KEYS` (decrypt fallbacks); no ciphertext-format change, AEAD-safe | `crates/maidan-auth/src/peer_secret.rs` |

## v188.0.0 — SaaS ops: per-workspace usage / metering

| Change | Where |
|--------|-------|
| `GET /workspaces/:id/usage` (workspace:read) returns live member/channel/thread/message counts (tombstones excluded); a low-cardinality metering basis (on-demand DB aggregate, not per-tenant Prometheus series) | `maidan-types/src/usage.rs` + `maidan-store` + `routes/workspace.rs` |

## v187.0.0 — SaaS ops: workspace export / portability

| Change | Where |
|--------|-------|
| `GET /workspaces/:id/export` (token:admin) returns the workspace content graph (members, channels+members, threads, messages+edits, pins, references) as one JSON bundle; secrets + ops tables excluded | `crates/maidan-server/src/export.rs` + `routes/workspace.rs` |

## v186.0.0 — SaaS ops: data-retention pruning

| Change | Where |
|--------|-------|
| Opt-in age retention for the event log (floored at `min_delivery_cursor`), audit trail, and delivery tables; batched background sweeper + `MAIDAN_RETENTION_*` config + `maidan_retention_pruned_total` | `maidan-store/src/{sqlite,postgres}/retention.rs` + `maidan-server/src/retention.rs` |

## v185.0.0 — SaaS ops: Helm hardening (probes, PDB, NetworkPolicy, existingSecret)

| Change | Where |
|--------|-------|
| Liveness/startup → shallow `/health/live` (restart-storm fix), readiness → deep `/health/ready`; opt-in `PodDisruptionBudget` (on in prod) + `NetworkPolicy`; `existingSecret` support | `helm/maidan/` |

## v184.0.0 — Correctness: harden the domain-write → event-append dual write

| Change | Where |
|--------|-------|
| `publish()` retries the durable event append on transient errors, splits append-failure (lost event, loud + metered via `maidan_event_append_failures_total`) from benign bus-publish failure | `crates/maidan-server/src/{routes/mod,metrics}.rs` |

## v183.0.0 — Security: default-on rate limit + explicit request body cap

| Change | Where |
|--------|-------|
| Built-in global per-client rate limit (1200 req/60s) when `MAIDAN_RATE_LIMIT_MAX` unset (server-binary only; explicit env incl. `0` overrides) | `crates/maidan-server/src/{rate_limit/mod,state,main}.rs` |
| Explicit env-tunable request body cap (`MAIDAN_MAX_BODY_BYTES`, default 2 MiB); oversized body → `413` | `crates/maidan-server/src/{app,error}.rs` |

## v182.0.0 — Security: audit-log coverage for credential + membership mutations

| Change | Where |
|--------|-------|
| Audit trail now records `token.mint`/`token.revoke` (incl. OIDC first-admin), `app_token.mint`/`app_installation.revoke`, `channel_member.add`/`.remove`, `message.purge` — best-effort writes via `crate::audit::record`; table-level 401/403 denial auditing deliberately excluded (write-amplifier → logs/metrics) | `crates/maidan-server/src/audit.rs` + token/apps/channel/message/session handlers |

## v181.0.0 — Correctness: one EventKind parser, round-trip guarded

| Change | Where |
|--------|-------|
| Store `parse_kind` (both backends) delegates to the single `EventKind::parse` — no per-backend copy to drift (the Cluster 171 silent-rollback bug class); `EventKind::ALL` + round-trip guard with a compile-time tripwire on new variants | `crates/maidan-types/src/events.rs` + `maidan-store/src/{sqlite,postgres}/events.rs` |

## v180.0.0 — Security: DM-thread access is participant-checked everywhere

| Change | Where |
|--------|-------|
| `ensure_thread_access` is DM-participant-aware (new `ensure_dm_participant` + `can_access_thread`); generic thread/message/social routes + A2A ingress gate on it; search + workspace-context filter per-thread — closes DM read/write/leak via the `__dm__` channel exemption | `crates/maidan-auth/src/access.rs` + route/tool gates |

## v179.0.0 — Security: A2A ingress channel/thread RBAC

| Change | Where |
|--------|-------|
| `POST /a2a/v1/rpc` enforces `ensure_channel_access` on post + task-read (closes a private-channel bypass the 160–165 RBAC arc missed) | `crates/maidan-server/src/a2a_agent.rs` |

## v178.0.0 — Token: opt-in lean event frames

| Change | Where |
|--------|-------|
| `lean` subscribe flag (WS + MCP SSE) → event frames carry `{log_id, kind, ...ids}` pointers instead of full events | `crates/maidan-server/src/{event_stream,ws,mcp_stream}.rs` |

## v177.0.0 — Token: omit empty message metadata

| Change | Where |
|--------|-------|
| `Message.metadata` omitted from serialization when empty (`{}`/`null`) — REST, events, MCP, write-acks | `crates/maidan-types/src/models.rs` |

## v176.0.0 — Token: capability-filtered tools/list

| Change | Where |
|--------|-------|
| MCP `tools/list` returns only the tools the caller's capabilities allow (`catalog_for`); bypass sees all | `crates/maidan-mcp/src/tools/mod.rs` |

## v175.0.0 — Token: MCP search snippet_only parity

| Change | Where |
|--------|-------|
| MCP `search_messages` `snippet_only` (drop bodies, keep snippet) — parity with REST | `crates/maidan-mcp/src/tools/search.rs` |

## v174.0.0 — Agentic: human-in-the-loop approvals

| Change | Where |
|--------|-------|
| MCP `request_approval` — server→client `elicitation/create` HITL gate; returns `{approved, action, content}` | `crates/maidan-mcp/src/tools/approval.rs` |

## v173.0.0 — Agentic: structured message content

| Change | Where |
|--------|-------|
| Typed `content` blocks on messages (`text`/`code`/`tool_use`/`tool_result`/`resource_link`), REST + MCP, both backends; `body` derived when omitted | `crates/maidan-types/src/models.rs`, `crates/maidan-store/src/{postgres,sqlite}/messages.rs` |
| `content` column on `maidan_messages` (pg `0034` JSONB / sqlite `0033` TEXT) | `migrations/*/00xx_message_content.sql` |

## v172.0.0 — Agentic: MCP structured backpressure

| Change | Where |
|--------|-------|
| Rate-limited `POST /mcp` + `/mcp/streamable` return a JSON-RPC error envelope (`-32029` + `data.retry_after_ms`), still 429 + `Retry-After` | `crates/maidan-server/src/rate_limit/mod.rs` |
| `McpError::RateLimited { retry_after_ms }` | `crates/maidan-mcp/src/error.rs` |

## v171.0.0 — Agentic: thread task assignment / handoff

| Change | Where |
|--------|-------|
| `Thread.assignee_id` axis + `assign` / atomic `claim` / `unassign` (both backends) | `crates/maidan-store/src/{postgres,sqlite}/threads.rs` |
| REST `PUT`/`DELETE /threads/:id/assignee` + `POST …/assignee/claim` (`thread:transition`, RBAC-gated) | `crates/maidan-server/src/routes/thread.rs` |
| MCP `assign_thread` / `claim_thread` / `unassign_thread` | `crates/maidan-mcp/src/tools/thread.rs` |
| `ThreadAssignmentChanged` event (prev→new assignee + actor) | `crates/maidan-types/src/events.rs` |

## v170.0.0 — CI/CD: native arm64 release build + trivy image scan

| Change | Where |
|--------|-------|
| arm64 `maidan-server` image builds on a native `ubuntu-24.04-arm` runner (no QEMU) — kills the ~2 h emulated Rust compile | `.github/workflows/release.yml` |
| trivy vulnerability scan of the released server image (report-only) | `.github/workflows/release.yml` |

## v169.0.0 — Perf: coalesce optimistic delivery-cursor writes

| Fix | Where |
|-----|-------|
| Optimistic subscribe path buffers the delivery cursor (persist per 64 events / 500 ms + flush on stream end) instead of a DB write per event; lag-replay advances once to the batch high-water | `crates/maidan-server/src/event_stream.rs` |

## v168.0.0 — Perf: outbox relay round-trips + tunable broadcast cap

| Fix | Where |
|-----|-------|
| Outbox `list_pending` JOINs the event payload; relay publishes from it (no per-row `get_stored_event`) + batch `mark_published_batch` | `crates/maidan-store/src/{postgres,sqlite}/outbox.rs`, `crates/maidan-server/src/outbox_relay.rs` |
| Env-tunable broadcast capacity `MAIDAN_BUS_BROADCAST_CAP` (event bus + presence/resource notifiers) | `crates/maidan-bus/src/lib.rs` |
| Hotfix: removed two `unwrap()`s in the webhook worker (Cluster 166) that failed the strict lint | `crates/maidan-server/src/webhook_worker.rs` |

## v167.0.0 — Perf: rate-limiter map eviction + embedding model cache

| Fix | Where |
|-----|-------|
| Rate-limiter in-memory bucket map bounded (evict elapsed windows) | `crates/maidan-server/src/rate_limit/limiter.rs` |
| `PostgresSearch` caches model→table (skips SELECT + create-checks per upsert) | `crates/maidan-search/src/postgres.rs` |

_Post-gate hardening (Phase XXIV): arc 2 (perf), part 2 — a memory leak + the embedding-upsert round-trip halving. No new gate tag._

## v166.0.0 — Perf: per-connection SQLite pragmas + per-workspace webhook fan-out

| Fix | Where |
|-----|-------|
| SQLite `foreign_keys`/`busy_timeout`/WAL in `after_connect` (per connection) | `crates/maidan-search/src/sqlite_vec.rs` (`pool_options_with`) |
| Webhook fan-out queries only the event's workspace (was an all-workspaces scan) | `crates/maidan-server/src/webhook_worker.rs`, store `list_enabled_webhook_subscriptions_for_workspace` |

_Post-gate hardening (Phase XXIV): arc 2 (perf + CI/CD), part 1 — a real SQLite correctness bug + the biggest per-event query win. No new gate tag._

## v165.0.0 — Reference authorization (RBAC arc complete)

| Capability | Where |
|------------|-------|
| `create`/`list_references` (REST) + `add_reference` (MCP) gated on the referenced entity's channel access | `crates/maidan-server/src/routes/reference.rs`, `crates/maidan-mcp/src/tools/mod.rs` |

_Post-gate hardening (Phase XXIV): final RBAC cluster. References resolve Thread/Message → channel access (also fixes a missing workspace check). **The channel/thread RBAC arc (159–165) is complete.** No new gate tag._

## v164.0.0 — channel:admin membership API (RBAC part F)

| Capability | Where |
|------------|-------|
| `channel:admin` cap + `/channels/:cid/members` REST (add/list/remove) | `crates/maidan-server/src/routes/channel.rs`, `app.rs`, `openapi` |
| MCP `add_channel_member` / `list_channel_members` / `remove_channel_member` | `crates/maidan-mcp/src/tools/channel.rs` + catalog + contracts |

_Post-gate hardening (Phase XXIV): sixth RBAC cluster. Makes private channels operational — admins grant/revoke membership. No new gate tag._

## v163.0.0 — Verified WS/MCP subscribe grants (RBAC part E)

| Capability | Where |
|------------|-------|
| Subscribe `channel_grants` verified against `channel_is_member` (private-channel events gated) | `crates/maidan-server/src/subscribe_grants.rs`, `ws.rs`, `mcp_stream.rs` |

_Post-gate hardening (Phase XXIV): fifth RBAC cluster. Closes the private-channel event leak on WS + MCP SSE (asserted grants were previously trusted). No new gate tag._

## v162.0.0 — MCP aggregate-read filtering (RBAC part D)

| Capability | Where |
|------------|-------|
| `search_messages` / `list_channels` / `get_workspace_context` filter private-channel content by access | `crates/maidan-mcp/src/tools/{search,channel,mod}.rs` |

_Post-gate hardening (Phase XXIV): fourth RBAC cluster. Closes the MCP aggregate-read leaks; with 160+161 the channel-content read/write vuln is closed on REST + MCP. No new gate tag._

## v161.0.0 — Private-channel access control over MCP (RBAC part C)

| Capability | Where |
|------------|-------|
| MCP pre-dispatch per-channel gate for point-access content tools | `crates/maidan-mcp/src/tools/mod.rs` (`enforce_channel_access`) |
| `resources/read` gates `threads/{id}` + `channels/{id}` | `crates/maidan-mcp/src/server.rs` |

_Post-gate hardening (Phase XXIV): third RBAC cluster. Closes the MCP read/write path into private channels (aggregate reads — search / workspace-context / list-channels — filtered next). No new gate tag._

## v160.0.0 — Private-channel access control over REST (RBAC part B)

| Capability | Where |
|------------|-------|
| `ensure_channel_access` / `ensure_thread_access` / `ensure_message_access` / `can_access_channel` | `crates/maidan-auth/src/access.rs` |
| Per-channel enforcement on all REST content routes + search + workspace-context | `crates/maidan-server/src/routes/{channel,thread,message,social,search,workspace}.rs` |

_Post-gate hardening (Phase XXIV): second RBAC cluster. Private channels require a `channel_members` row; public + `__dm__` unchanged; creator auto-added on private create. Closes the workspace-flat read/write vuln on REST. MCP + subscribe + references follow. No new gate tag._

## v159.0.0 — Channel membership model (RBAC part A)

| Capability | Where |
|------------|-------|
| `channel_members` table + `ChannelMember`/`ChannelMemberRole` + 4 Store methods (both backends) | `crates/maidan-store/src/{postgres,sqlite}/channel_members.rs`, migrations `0032`/`0031` |

_Post-gate hardening (Phase XXIV): first cluster of the flagship channel/thread RBAC. Membership substrate only — additive, no enforcement (Cluster 160), zero behavior change. No new gate tag._

## v158.0.0 — Signed container images (keyless cosign)

| Capability | Where |
|------------|-------|
| `cosign sign` (keyless) on the `maidan-server` + `maidan-postgres` images, by digest | `.github/workflows/release.yml` (`sign-images` job) |

_Post-gate hardening (Phase XXIV): enterprise-hardening arc part 3. Closes the unsigned-images supply-chain gap; images are verifiable in an admission controller. Runs on the release tag. No new gate tag._

## v157.0.0 — Fail-closed `AUTH_DISABLED`

| Capability | Where |
|------------|-------|
| `AUTH_DISABLED` requires explicit `MAIDAN_ALLOW_INSECURE_NO_AUTH` ack + never in prod (refuses boot otherwise) | `crates/maidan-server/src/{config,auth}.rs` |

_Post-gate hardening (Phase XXIV): enterprise-hardening arc part 2. Closes the silent-open-door risk (`AUTH_DISABLED` alone in a non-prod/unset-env deployment). Coordinated across compose/helm CI manifests. No new gate tag._

## v156.0.0 — Production-safety defaults (SIGTERM drain + statement timeout)

| Capability | Where |
|------------|-------|
| SIGTERM graceful shutdown (k8s/systemd drain) | `crates/maidan-server/src/main.rs` |
| Default 30 s `statement_timeout` (runaway-query cap) | `crates/maidan-server/src/config.rs` |

_Post-gate hardening (Phase XXIV): first cluster of the enterprise-hardening arc (from the 5-agent production-readiness sweep). Safe-by-default; both are configurable. No new gate tag._

## v155.0.0 — Sampling-backed `summarize_thread` (first `request_client` caller)

| Capability | Where |
|------------|-------|
| MCP `summarize_thread` — asks the connected client to sample a thread summary (server→client `sampling/createMessage` over the GET stream) | `crates/maidan-mcp/src/tools/thread.rs`, catalog + contracts |
| Tool dispatch carries the streamable session id (`handle_in_session`) | `crates/maidan-mcp/src/server.rs`, `crates/maidan-server/src/mcp_streamable.rs` |

_Post-gate hardening (Phase XXIV): closes arc lane 3 and the three-lane next-arc plan (token efficiency 151+152, live UI 153, request_client 154+155). `request_client` now has a real in-tree caller. No new gate tag._

## v154.0.0 — `request_client` GET-stream delivery

| Capability | Where |
|------------|-------|
| Server→client requests (sampling/roots/elicitation) delivered on the canonical `GET /mcp/streamable` | `crates/maidan-mcp/src/streamable_session.rs`, `crates/maidan-server/src/mcp_streamable.rs` |

_Post-gate hardening (Phase XXIV): arc lane 3, part 1. Per-session request broadcast + GET-stream merge; POST-leg mpsc/replay untouched. A real caller (sampling-backed `summarize_thread`) arrives in Cluster 155. No new gate tag._

## v153.0.0 — Live-updating `/ui` thread view

| Capability | Where |
|------------|-------|
| `/ui` thread view refreshes live from WS message/reaction/pin frames (debounced) | `crates/maidan-server/static/index.html` |

_Post-gate hardening (Phase XXIV): UI polish (arc lane 2). Routes the WS domain-event frames — previously only Events-tab log lines — into `loadMessages` for the open thread. No backend change._

## v152.0.0 — Lean HTTP context pack + snippet-only search

| Capability | Where |
|------------|-------|
| HTTP `/threads/:id/context` + `/workspaces/:wid/context` edits lean by default (`MessageEditView`, optional bodies), opt-in `include_edits=true` | `crates/maidan-server/src/thread_context.rs` |
| `GET /workspaces/:wid/search?snippet_only=true` drops full bodies (semantic hits get a truncated snippet) | `crates/maidan-server/src/routes/search.rs`, `crates/maidan-search/src/hit.rs` |

_Post-gate hardening (Phase XXIV): token-efficiency part 2 (arc item B1), extending Cluster 151's MCP lean reads to REST. Both context-pack surfaces + search now have opt-in token-lean modes. No new gate tag._

## v151.0.0 — Token-efficient lean context reads

| Capability | Where |
|------------|-------|
| `get_thread_context` edits lean by default (`{id, editor, edited_at}`), opt-in `include_edits=true` for full bodies | `crates/maidan-mcp/src/context.rs` |
| `list_messages` limit clamped to `1..=500` | `crates/maidan-mcp/src/tools/message.rs` |

_Post-gate hardening (Phase XXIV): first token-efficiency cluster (arc item B1). Edit bodies were the largest token cost in a context pack; `get_workspace_context` inherits the lean default through its nested packs. MCP-only; the typed HTTP `/threads/:id/context` pack is a deferred follow-up. No new gate tag._

## v150.0.0 — MCP stream thread/member/kind filters

| Capability | Where |
|------------|-------|
| `GET /mcp/stream` narrowing by `channel_id`/`thread_id`/`member_id`/`kinds` (await my mention) | `crates/maidan-server/src/mcp_stream.rs` |

_Post-gate hardening (Phase XXIV): completes the MCP-agent-surface pair (149 discover + 150 await mentions). Pure query→filter wiring over the existing `EventFilter`; no new gate tag._

## v149.0.0 — MCP inbox + mention tools

| Capability | Where |
|------------|-------|
| MCP `list_mentions` / `get_inbox` / `mark_inbox_read` (agent discovers its @mentions) | `crates/maidan-mcp/src/tools/member.rs`, catalog + contracts |

_Post-gate hardening (Phase XXIV): first of the MCP-agent-surface arc (149–150), from the next-arc research. Closes the gap where an MCP-only agent couldn't see it was @mentioned. No new gate tag._

## v148.0.0 — MCP server→client requests (streamable arc complete)

| Capability | Where |
|------------|-------|
| Server→client JSON-RPC requests (sampling / roots / elicitation), capability-gated + correlated | `maidan-mcp/src/server.rs::request_client`, `streamable_session.rs` |
| Per-session client-capability tracking (from `initialize`) | `mcp_streamable.rs`, `streamable_session.rs` |

_Post-gate hardening (Phase XXIV): concludes the MCP streamable spec-completeness arc (145–148) — version negotiation, header, batching, notifications, GET SSE, `Accept`, resumability, and now bidirectional requests. No new gate tag; the backlog item is closed._

## v147.0.0 — MCP streamable resumability (Last-Event-ID)

| Capability | Where |
|------------|-------|
| SSE `id:` on session frames + `Last-Event-ID` reconnect replay | `maidan-mcp/src/streamable_session.rs`, `mcp_streamable.rs` |
| Streamable session survives a dropped POST leg (reconnectable) | `mcp_streamable.rs` |

_Post-gate hardening (Phase XXIV): part 3 of the MCP streamable spec-completeness arc (145–148). Server→client requests (148) remain. No new gate tag._

## v146.0.0 — MCP GET /mcp/streamable SSE + Accept negotiation

| Capability | Where |
|------------|-------|
| `GET /mcp/streamable` server→client SSE stream (session-aware) | `mcp_streamable.rs::stream_get`, `app.rs`, cap-map |
| `Accept`-based JSON-vs-SSE content negotiation on `POST /mcp/streamable` | `mcp_streamable.rs::accepts_event_stream` |

_Post-gate hardening (Phase XXIV): part 2 of the MCP streamable spec-completeness arc (145–148). Resumability (147) and server→client requests (148) remain. No new gate tag._

## v145.0.0 — MCP conformance basics (initialize/version + batching + notifications)

| Capability | Where |
|------------|-------|
| MCP `initialize` protocol-version negotiation; `MCP-Protocol-Version` header validation | `maidan-mcp/src/server.rs`, `maidan-server/src/mcp.rs`, `mcp_streamable.rs` |
| JSON-RPC batching + notifications (`202`) on `POST /mcp` | `maidan-server/src/mcp.rs` |

_Post-gate hardening (Phase XXIV): first of the MCP streamable spec-completeness arc (145–148). Closes the JSON-RPC/lifecycle conformance gaps; streamable-transport gaps (GET SSE, resumability, server→client requests) follow in 146–148. No new gate tag._

## v144.0.0 — Docs dead-link gate + latent-link cleanup

| Capability | Where |
|------------|-------|
| CI fails the docs build on dead internal links (was: shipped silently) | `book/book.toml` `[output.linkcheck]`, `.github/workflows/docs.yml`, `book/sync-docs.sh` |
| 35 latent broken published links fixed; space-files hyphenated (cleaner URLs) | `book/sync-docs.sh`, `book/src/SUMMARY.md` |

_Post-gate hardening (Phase XXIV): the 141 follow-up — turns the doc-nav guarantee into a CI gate and fixes the broken links it surfaced. Backlog docs reconciled (132 audit API + 134–143 UI track). No new gate tag._

## v143.0.0 — Richer message rendering (timestamps + slash results)

| Capability | Where |
|------------|-------|
| Thread messages show `posted_at` + inline slash-command results | `static/index.html` (`renderMessages`/`renderSlashResult`) |

_Post-gate hardening (Phase XXIV): UI-only polish surfacing data already in the message payload; completes the slash loop in the thread view. No new gate tag._

## v142.0.0 — Slash-command registry in the console

| Capability | Where |
|------------|-------|
| Register / list / revoke slash commands in `/ui` (new "Slash" tab) | `static/index.html`, `/ui/api/workspaces/:wid/slash-commands[/:cid]` |

_Post-gate hardening (Phase XXIV): surfaces the slash-command registry reusing the tested `slash_commands::*` handlers under `/ui/api`; one-time secret display for `http` handlers. Execution stays message-triggered (`/name args`). No new gate tag._

## v141.0.0 — Published docs serve every page (dead-nav fix)

| Capability | Where |
|------------|-------|
| The mdBook site builds + serves all 21 SUMMARY pages (was ~20 dead links) | `book/sync-docs.sh`, `book/src/SUMMARY.md`, `.github/workflows/docs.yml` |
| Landing-page quickstart + helpful custom 404 | `book/src/introduction.md`, `book/src/404.md` |

_Post-gate hardening (Phase XXIV): a build-time staging step copies the canonical `docs/*` into `book/src/docs/` so mdBook builds them as real in-site pages; the integration guide is now reachable from the live nav. No new gate tag._

## v140.0.0 — Workspace presence roster in the console

| Capability | Where |
|------------|-------|
| Live presence roster + online/away in `/ui` (over the WS) | `static/index.html` (`renderPresence`/`setPresence`) |

_Post-gate hardening (Phase XXIV): renders the realtime `presence_snapshot` frames (already on the WS) into a roster; no backend change — presence is WS-only. No new gate tag._

## v139.0.0 — 1:1 direct messages in the console

| Capability | Where |
|------------|-------|
| Open / list / read / post 1:1 DMs in `/ui` (new "DMs" tab) | `static/index.html`, `/ui/api/workspaces/:wid/dm`, `/ui/api/dm/:id/messages` |

_Post-gate hardening (Phase XXIV): a new `/ui` view reusing the tested `dm::*` handlers under `/ui/api`; the conversation pane reads via the existing thread-messages route (DMs are thread-backed). The exact parallel to group DMs (136). No new gate tag._

## v138.0.0 — Global audit + reindex controls (operator console complete)

| Capability | Where |
|------------|-------|
| Load cross-workspace global audit in `/ui` (bearer, `audit:read-global`) | `static/index.html`, top-level `/operator/audit` |
| Trigger + poll embedding reindex in `/ui` (workspace = session; global = `token:admin`) | `static/index.html`, `/ui/api/operator/reindex-embeddings[/:job_id]` |

_Post-gate hardening (Phase XXIV): completes the "Operator" tab (137 + 138). Each control is gated by the cap it actually needs and degrades honestly without a token. No new gate tag._

## v137.0.0 — Deliveries & DLQ in the operator console

| Capability | Where |
|------------|-------|
| List + replay webhook/automation deliveries (incl. DLQ) in `/ui` (new "Operator" tab) | `static/index.html`, `/ui/api/workspaces/:wid/deliveries[/:did/replay]` |

_Post-gate hardening (Phase XXIV): a new `/ui` view reusing the tested `delivery_ops` handlers under `/ui/api`; list (`workspace:read`) + replay (`workspace:write`) map onto the operator-session caps, so it works on a plain login. No new gate tag._

## v136.0.0 — Group DMs in the operator console

| Capability | Where |
|------------|-------|
| Open / list / read / post group DMs in `/ui` (new tab) | `static/index.html`, `/ui/api/.../group-dms` |

_Post-gate hardening (Phase XXIV): a new `/ui` view reusing the tested group-DM handlers under `/ui/api`; the conversation pane reads via the existing thread-messages route (group DMs are thread-backed). No new gate tag._

## v135.0.0 — Pins in the thread view

| Capability | Where |
|------------|-------|
| Pin/unpin in `/ui` (per-message toggle) | `static/index.html`, `/ui/api/threads/:tid/pins` |

_Post-gate hardening (Phase XXIV): pins affordance reusing the tested pin handlers under `/ui/api`. No new gate tag._

## v134.0.0 — Reactions in the operator UI

| Capability | Where |
|------------|-------|
| Emoji reactions in `/ui` (chips, quick-add, toggle) | `static/index.html`, `/ui/api/messages/:mid/reactions` |

_Post-gate hardening (Phase XXIV): first UI feature on the repaired/guarded base — reuses the tested reaction handlers under `/ui/api`. No new gate tag._

## v133.0.0 — /ui write-path repair + JS guard

| Capability | Where |
|------------|-------|
| `/ui` write path works (session or bearer); undefined-helper CI guard | `crates/maidan-server/static/index.html`, `tests/ui_js_contract.rs` |

_Post-gate hardening (Phase XXIV): repaired a shipped-broken, CI-invisible `/ui` write path (4 undefined JS refs) and added a guard so the bug class fails CI. Foundation for the UI feature clusters. No new gate tag._

## v132.0.0 — Global admin audit query API

| Capability | Where |
|------------|-------|
| `GET /operator/audit` — cross-workspace audit, gated by `audit:read-global` | `routes/workspace.rs::list_global_audit`, `maidan-auth` capability |

_Post-gate hardening (Phase XXIV): exposes the existing cross-workspace `Store::list_audit` behind a new global capability (no org model needed). Completes the 127–132 sweep. No new gate tag._

## v131.0.0 — Delivery-unification verification-close

| Capability | Where |
|------------|-------|
| Webhook + automation delivery unified (logic + operator API; storage intentionally separate) | `automation_delivery.rs`, `webhooks.rs`, `delivery_ops.rs` |

_Post-gate hardening (Phase XXIV): docs-only. Verified the unify-delivery item substantially addressed and declined a risky storage-table migration; rationale recorded. No new gate tag._

## v130.0.0 — Test-coverage uplift (observability + MCP)

| Capability | Where |
|------------|-------|
| Tested observability env-parsing (pure parsers) | `crates/maidan-observability/src/{metrics,lib}.rs` |
| MCP prompts catalog-integrity test | `crates/maidan-mcp/src/prompts.rs` |

_Post-gate hardening (Phase XXIV): fills the zero-coverage gaps the v126 scan named, via race-free pure-function refactors. No new gate tag._

## v129.0.0 — Hardening: error-visibility + bounded buffers

| Capability | Where |
|------------|-------|
| Bounded MCP streamable session buffer (no memory-exhaustion) | `crates/maidan-mcp/src/streamable_session.rs` |
| Outbox quarantine-failure visibility (no silent infinite-retry) | `crates/maidan-server/src/outbox_relay.rs` |
| Request-handler `unreachable!()` → typed errors | `delivery_ops.rs`, `crates/maidan-mcp/src/resources.rs` |

_Post-gate hardening (Phase XXIV): the top correctness/robustness findings from the v126 scan. No new gate tag._

## v128.0.0 — A2A delivery robustness

| Capability | Where |
|------------|-------|
| A2A push retry + backoff + `maidan_a2a_push_total` metric | `crates/maidan-server/src/a2a_agent.rs` |
| A2A client connect/request timeouts (no indefinite hang) | `crates/maidan-a2a/src/client.rs` |

_Post-gate hardening (Phase XXIV): the A2A delivery paths were fire-and-forget (no timeout/retry/logging); now bounded, retried, and observable. No new gate tag._

## v127.0.0 — Backlog reconciliation

| Capability | Where |
|------------|-------|
| Backlog verified against code (v126) — trustworthy open-work list | `docs/Remaining Work.md`, `docs/Open Work.md` |

_Post-gate hardening (Phase XXIV): docs-only — corrected ~11 phantom (already-shipped) backlog entries + the stale `Open Work` tail, so the remaining-work list matches the code. No new gate tag._

## v126.0.0 — MCP SSE at-least-once parity

| Capability | Where |
|------------|-------|
| At-least-once on MCP SSE (`/mcp/stream?at_least_once=true`) | `crates/maidan-server/src/mcp_stream.rs` (reuses `event_stream::reconcile_deliver`) |

_Post-gate hardening (Phase XXIV): extends the Cluster 125 at-least-once delivery to the MCP SSE transport — both real-time transports now offer opt-in gap-free delivery. No new gate tag._

## v125.0.0 — At-least-once event delivery

| Capability | Where |
|------------|-------|
| Opt-in at-least-once subscribe (gap-free, in-order, per-consumer) | `at_least_once` flag (`/ws/subscribe`), `event_stream::reconcile_deliver` |
| Stability-gated gap-safe event replay | `Store::list_events_after_stable`, `maidan_events.inserted_at` |

_Post-gate hardening (Phase XXIV): closes the silent out-of-order delivery gap with an opt-in cursor-driven reconcile mode (time-based stability horizon); the default optimistic low-latency path is unchanged. No new gate tag._

## v124.0.0 — CI / observability loose ends

| Capability | Where |
|------------|-------|
| Single SLO-rule validator (promtool check + unit tests) | `scripts/check-alert-rules.sh` |
| 8 required status checks (adds `promtool (alert rules)` + `otlp smoke`) | branch protection on `main`; [[Operations]] |

_Post-gate hardening (Phase XXIV): collapses the two overlapping rule validators into one and promotes the Cluster 122/123 observability jobs to required checks. No new gate tag._

## v123.0.0 — OTLP delivery proven end-to-end

| Capability | Where |
|------------|-------|
| OTLP traces + metrics asserted against a real collector in CI | `compose.yaml` (`otlp` profile), `docker/otel-collector-config.yaml`, `scripts/otlp-smoke.sh`, `.github/workflows/ci.yml` (`otlp smoke`) |

_Post-gate hardening (Phase XXIV): closes the residual observability gap from Cluster 122 — the OTLP export wiring (Cluster 89) is now proven against a running collector, not just an in-process unit test. No new gate tag._

## v122.0.0 — Alert rules executed in CI

| Capability | Where |
|------------|-------|
| SLO recording/alert PromQL executed in CI (`check rules` + unit tests) | `.github/workflows/ci.yml` (`promtool (alert rules)`), `scripts/check-alert-rules.sh` |
| SLO rule unit tests (queue-sat guard, embed-failure restart-safety, `$value`) | `docs/alerts/prometheus-rules-maidan-slo.test.yaml` |

_Post-gate hardening (Phase XXIV): closes the "alert exprs never executed" gap from Cluster 121 — which immediately caught a `$value`-rendering bug in `MaidanIndexerQueueSaturated`. Also corrects the OTLP-export status (shipped in Cluster 89). No new gate tag._

## v121.0.0 — Observability & contract completeness

| Capability | Where |
|------------|-------|
| Every OpenAPI op classified (bearer / session / public) in CI | `crates/maidan-server/tests/http_openapi_capability_map_contract.rs` |
| Indexer queue-saturation recording rule + backpressure/embed-failure alerts | `docs/alerts/prometheus-rules-maidan-slo.yaml` |
| Operator dashboard panels for indexer queue depth + embed failures | `docs/dashboards/maidan-operator.json` |

_Post-gate hardening (Phase XXIV): closes the OpenAPI-wide capability-map gap (Cluster 69) and extends the Cluster 90 SLO surface to the Cluster 116 indexer metrics. No new gate tag._

## v120.0.0 — Scale product gate (`maidan-scale-1.0`)

| Capability | Where |
|------------|-------|
| `maidan-scale-1.0` gate (criteria → evidence) | `docs/Gates/maidan-scale-1.0.md`, `maidan_scale_gate_e2e` |
| Recorded store bench baseline | `crates/maidan-store/benches/STORE_BASELINE.md` |
| `scale-out smoke` as a gate-required check | `.github/workflows/ci.yml` |

_Closes Product Ladder 102+ (gate **`maidan-scale-1.0`** at **`v120.0.0`**)._

## v119.0.0 — Dependency dedupe & currency

| Capability | Where |
|------------|-------|
| Duplicate-major CI gate (`multiple-versions = deny`) | `deny.toml` (`lint` job) |
| Dependency currency + duplicate-version policy doc | `docs/Dependencies.md` |
| Workspace on thiserror 2 | `Cargo.toml` |

## v118.0.0 — Hybrid relevance

| Capability | Where |
|------------|-------|
| Hybrid lexical+semantic search (HTTP + MCP) | `crates/maidan-server/src/routes/search.rs`, `crates/maidan-mcp/src/tools/search.rs` |
| Score fusion (`fuse_hybrid`, `DEFAULT_HYBRID_WEIGHT`) | `crates/maidan-search/src/score.rs`, `traits.rs` |
| Relevance eval harness | `crates/maidan-search/tests/relevance_eval.rs` |

## v117.0.0 — Pluggable production provider

| Capability | Where |
|------------|-------|
| Production `openai-compatible` embeddings with auto-detected dimension | `crates/maidan-search/src/embedding_provider.rs` |
| Boot-time per-model registration (`Search::ensure_model`) | `crates/maidan-search/src/traits.rs`, `postgres.rs`, `sqlite.rs` |
| Embedding provider + model-migration guide | `docs/Embeddings.md` |

## v116.0.0 — Batch embedding pipeline

| Capability | Where |
|------------|-------|
| Batched live embedding indexer (bounded queue + backpressure) | `crates/maidan-search/src/embedding_batcher.rs` |
| Batch embedding provider API (`embed_batch`) | `crates/maidan-search/src/embedding_provider.rs` |
| Chunked large-workspace backfill | `crates/maidan-search/src/reindex.rs` |
| Bounded indexer-lag + throughput metrics | `crates/maidan-server/src/metrics.rs` (`maidan_indexer_queue_depth`, …) |

## v115.0.0 — Module split + `unwrap()` purge

| Capability | Where |
|------------|-------|
| No non-test `unwrap()`/`expect()` in `crates/*/src` (clippy-enforced) | `.github/workflows/ci.yml` (lint job) |
| Domain-organized HTTP route modules | `crates/maidan-server/src/routes/` |
| Domain-organized MCP tool modules | `crates/maidan-mcp/src/tools/` |

## v114.0.0 — Coverage uplift + envelope fuzz

| Capability | Where |
|------------|-------|
| Full-suite coverage gate (≥ 40% lines) | `.github/workflows/ci.yml` (`coverage` job) |
| JSON-RPC / MCP / A2A envelope round-trip + fuzz coverage | `maidan-mcp/src/{protocol,error}.rs`, `maidan-a2a/src/protocol.rs` |

## v113.0.0 — Backend parity harness

| Capability | Where |
|------------|-------|
| Migration + store-module lockstep guard (allowlisted) | `maidan-store/tests/backend_parity.rs` |
| Cross-dialect identity over FSM / edit / reaction surface | `maidan-store/tests/{common/mod.rs,dialect_parity.rs}` |

## v112.0.0 — FSM property tests

| Capability | Where |
|------------|-------|
| FSM transition + rank invariants under arbitrary inputs | `maidan-fsm/tests/fsm_properties.rs` |
| Hierarchical (tree-wide) rank-rule guarantee | `maidan-fsm/tests/fsm_properties.rs` (`locally_valid_tree_is_globally_consistent`) |

## v111.0.0 — `maidan-auth` test suite

| Capability | Where |
|------------|-------|
| Capability-vocabulary + `AuthContext` authorization matrix coverage | `maidan-auth/tests/capability_matrix.rs` |
| Peer-secret AEAD round-trip / tamper / key-parse coverage | `maidan-auth/tests/peer_secret_aead.rs` |
| Bearer lifecycle (mint / revoke / expire / forge) coverage | `maidan-auth/tests/token_lifecycle.rs` |

## v110.0.0 — Per-workspace fairness

| Capability | Where |
|------------|-------|
| Per-workspace request-rate fairness | `rate_limit::middleware`, `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` (key `ws:{wid}`) |
| Noisy-neighbor regression guard | `tenant_fairness_e2e` |

## v109.0.0 — ANN index tuning + search bench

| Capability | Where |
|------------|-------|
| Tunable HNSW build + query params | `hnsw::HnswParams`, `ensure_model_postgres`, `PostgresSearch::semantic_search` |
| Lexical + semantic latency bench + baseline | `maidan-search/benches/search_hot.rs`, `SEARCH_BASELINE.md` |

## v108.0.0 — Adaptive outbox relay

| Capability | Where |
|------------|-------|
| Drain-until-empty + idle backoff relay cadence | `OutboxRelay::run`, `RelayTick`, `backoff_step` |
| Prompt wake on enqueue (polling-safe mpsc nudge) | `AppState.outbox_nudge`, `OutboxRelay::with_nudge`, `wait_idle_or_nudge` |

## v107.0.0 — Configurable DB pool & timeouts

| Capability | Where |
|------------|-------|
| Env-tunable pool size + acquire timeout | `config::DbConfig`, `main.rs` |
| Postgres `statement_timeout` (migration-exempt) / SQLite `busy_timeout` | `after_connect` cap, `configure_sqlite_pool_with` |

## v106.0.0 — Bulk context reads

| Capability | Where |
|------------|-------|
| O(1)-query context assembly (no per-row N+1) | `thread_context.rs`, `Store::{list_threads_for_workspace, list_references_from_many, list_message_edits_for_messages}` |
| Query-count regression guard | `context_query_count_e2e` |

## v105.0.0 — Multi-replica scale-out smoke

| Capability | Where |
|------------|-------|
| Race-free boot migrations under N replicas | `run_postgres_migrations` advisory lock, `concurrent_migrations` test |
| Tested two-replica topology (shared PG + object store + LB) | `compose.yaml` `scale` profile, `scripts/scale-out-smoke.sh`, CI `scale-out smoke` |

## v104.0.0 — Durable ephemeral state

| Capability | Where |
|------------|-------|
| Durable single-use OAuth codes (any-replica exchange) | `maidan_oauth_codes`, `Store::{insert,consume}_oauth_code`, `app_oauth.rs` |
| Durable reindex job status (any-replica read) | `maidan_reindex_jobs`, `Store::{upsert,get}_reindex_job`, `reindex_ops.rs` |

## v103.0.0 — Distributed presence & roster

| Capability | Where |
|------------|-------|
| Cross-replica presence/typing fan-out | `maidan-bus::PresenceNotifier`, `PostgresPresenceNotifier` (`maidan_presence`) |
| Merged TTL roster across replicas | `PresenceHub` heartbeat + sweep, `AppState::attach_presence_notifier` |

## v102.0.0 — Cross-replica MCP resource notifications

| Capability | Where |
|------------|-------|
| Cross-process resource-update fan-out | `maidan-bus::ResourceNotifier`, `PostgresResourceNotifier` (`maidan_resource_updated`) |
| Per-replica notification delivery | `McpServer::spawn_resource_notify_listener`, `AppState::attach_resource_notifier` |

## v101.0.0 — Operator product gate

| Capability | Where |
|------------|-------|
| Operator gate e2e | `maidan_operator_gate_e2e.rs` |

## v100.0.0 — mcp-stdio embedded indexer

| Capability | Where |
|------------|-------|
| Stdio + in-process indexer | `maidan-cli` `mcp-stdio`, `McpServer::with_event_bus` |

## v99.0.0 — Presence v2 docs

| Capability | Where |
|------------|-------|
| Roster + WS presence guide | `docs/Presence and Roster.md` |

## v98.0.0 — Mention webhook router

| Capability | Where |
|------------|-------|
| Workspace mention webhook config | `mention_webhook_id`, `webhooks.rs` |

## v97.0.0 — Group DMs

| Capability | Where |
|------------|-------|
| Group DM (≥3 members) | migrations 0027/0028, `group_dm.rs` |

## v96.0.0 — /ui tokens & apps

| Capability | Where |
|------------|-------|
| List API tokens | `GET .../members/:mid/tokens` |
| UI token + app install list | `static/index.html` |

## v95.0.0 — /ui search

| Capability | Where |
|------------|-------|
| Faceted search tab | `/ui` search panel + `/ui/api/.../search` |

## v94.0.0 — /ui artifacts

| Capability | Where |
|------------|-------|
| Artifact cards + attach | `renderMessages`, upload flow |

## v93.0.0 — /ui live events

| Capability | Where |
|------------|-------|
| WS presets + reconnect + session subscribe | `index.html`, `ws.rs` |
| E2e | `ui_ws_tail_e2e.rs` |

## v92.0.0 — /ui channel browser

| Capability | Where |
|------------|-------|
| Session cookie writes on `/ui/api` | `POST` channels, threads, messages |
| Channel browser in static UI | `static/index.html` (`data-ui-version="6"`) |
| E2e | `ui_channels_e2e.rs` |

## v88.0.0 — Helm production profiles

| Capability | Where |
|------------|-------|
| OTel / Redis / S3 values overlays | `helm/maidan/values-profile-*.yaml` |
| Profile install guide | `helm/maidan/PROFILES.md` |
| Profile helm template smoke | `scripts/helm-template-smoke.sh` |

## v90.0.0 — SLO alert templates

| Capability | Where |
|------------|-------|
| Prometheus SLO rules + Alertmanager example | `docs/alerts/` |
| Rules validation script | `scripts/check-alert-rules.sh` (superseded the substring-only `validate-prometheus-rules.sh` in `v122.0.0`; now promtool check + unit tests) |
| Alert/metric contract test | `maidan-server/tests/alert_templates_contract.rs` |

## v89.0.0 — OTLP metrics export

| Capability | Where |
|------------|-------|
| OTLP metrics push (fanout with Prometheus) | `OTLP_METRICS`, `maidan-server::metrics`, `maidan-observability::metrics` |
| Example Grafana dashboard | `docs/dashboards/maidan-operator.json` |
| Helm otel profile enables metrics | `values-profile-otel.yaml` |

## v87.0.0 — Reindex job API

| Capability | Where |
|------------|-------|
| Operator reindex enqueue + poll | `POST/GET /operator/reindex-embeddings` |
| `Search::reindex_embeddings` | `maidan-search` Postgres + SQLite |
| Reindex job e2e | `maidan-server/tests/reindex_job_e2e.rs` |

## v86.0.0 — Per-model embedding query

| Capability | Where |
|------------|-------|
| `embedding_model` search param | `SearchQuery`, MCP `search_messages`, [[Production]] |
| Model-scoped semantic HTTP e2e | `search_semantic_e2e.rs` |

## v85.0.0 — sqlite-vec optional

| Capability | Where |
|------------|-------|
| Optional `sqlite-vec` feature | `maidan-search/Cargo.toml`, `maidan-server` feature `sqlite-vec` |
| CI linkage proof | `.github/workflows/ci.yml` job `sqlite-vec (optional feature)` |
| Brute-force SQLite semantic (default) | `SqliteSearch::semantic_search` without feature |

## v84.0.0 — Outbox relay modes

| Capability | Where |
|------------|-------|
| Polled outbox relay | `MAIDAN_OUTBOX_RELAY_MODE=polled`, `PostgresBusOptions` |
| Production outbox guard | `validate_startup` in `outbox_relay`, `MAIDAN_ENV=production` |
| SQLite outbox on by default | `main.rs` sqlite dialect |

## v83.0.0 — SQLite delivery cursor (ladder close)

| Capability | Where |
|------------|-------|
| SQLite delivery cursor | `maidan_delivery_cursor` migration `0023`, `SqliteStore::get/advance_delivery_cursor` |
| Cursor parity tests | `maidan-store/tests/delivery_cursor.rs` |

## v82.0.0 — Context pagination

| Capability | Where |
|------------|-------|
| Paginated thread context | `GET /threads/:id/context` (`message_cursor`, `next_message_cursor`) |
| Paginated workspace context | `GET /workspaces/:id/context` (`thread_cursor`, `next_thread_cursor`) |
| MCP context cursors | `get_thread_context` / `get_workspace_context` tool args |

## v81.0.0 — Subscribe grants v3

| Capability | Where |
|------------|-------|
| WS `channel_grants` | Subscribe frame filter; schema v3 |
| Private channel enforcement | `subscribe_grants`, `EventFilter::matches` |
| MCP stream grants | `GET /mcp/stream?channel_grants=…` |

## v79.0.0 — A2A long-running tasks

| Capability | Where |
|------------|-------|
| Task cancel | `tasks/cancel` on `POST /a2a/v1/rpc` |
| Subscribe progress | `SubscribeToTask` `statusUpdate` SSE frames |
| Terminal subscribe guard | JSON-RPC `-32005` |

## v80.0.0 — Delivery ops unified

| Capability | Where |
|------------|-------|
| Unified delivery list/get/replay | `GET/POST /workspaces/:wid/deliveries` |
| Webhook delivery operator store API | `list_webhook_deliveries`, `replay_webhook_delivery` |
| Automation routes (legacy) | `/workspaces/:wid/automation/deliveries` |

## v77.0.0 — HTTP capability map complete

| Capability | Where |
|------------|-------|
| Full HTTP capability map | `contracts/http-capability-map.json` |
| OpenAPI ↔ map CI | `http_openapi_capability_map_contract.rs` |
| HTTP deny matrix e2e | `http_capability_matrix_e2e.rs` |
| OpenAPI route parity | `openapi/paths/extensions.rs`, multipart stubs |

## v76.0.0 — Agent observability (`maidan-agent-1.0`)

| Capability | Where |
|------------|-------|
| Agent substrate gate e2e | `agent_substrate_gate_e2e.rs` |
| Ops runbook | [[Production#Agent observability]] |

## v72.0.0 — A2A task streaming

| Capability | Where |
|------------|-------|
| Persisted push config | `maidan_a2a_push_configs` |
| Persisted tasks | `maidan_a2a_tasks` |
| SubscribeToTask SSE | `POST /a2a/v1/rpc` |
| Push on task update | Best-effort POST to configured URL |

## v74.0.0 — MCP context export

| Capability | Where |
|------------|-------|
| `get_thread_context` | MCP `tools/call` |
| `get_workspace_context` | MCP `tools/call` |

## v71.0.0 — Subscribe contract v2

| Capability | Where |
|------------|-------|
| WS filter schema | `contracts/ws-subscribe-filter.schema.json` |
| EventKind forward-compat | [[Agent Integration]] |

## v70.0.0 — Vault truth pass

| Capability | Where |
|------------|-------|
| Architecture snapshot `v69` | [[Architecture]] |
| Reconciled backlog docs | [[Remaining Work]], [[Open Work]] |
| Agent integration README pitch | Root `README.md`, [[Agent Integration]] |

## v69.0.0 — Capabilities matrix complete

| Capability | Where |
|------------|-------|
| MCP tool → capability map | `contracts/mcp-capability-map.json` |
| MCP matrix e2e | `mcp_capability_matrix_e2e.rs` |
| HTTP capability contract | `contracts/http-capability-routes.json` |
| Contract CI | `scripts/check-agent-contract.sh` |

## v68.0.0 — Automation delivery guarantees

| Capability | Where |
|------------|-------|
| Automation delivery ledger | `maidan_automation_deliveries` (slash + FSM HTTP) |
| Retry worker | `maidan-server::automation_worker` |
| List / replay / DLQ | `GET/POST /workspaces/:wid/automation/*` |
| Slash sync-then-queue | `maidan-server::slash_commands` |
| FSM async HTTP dispatch | `maidan-server::fsm_hooks` |

## v67.0.0 — Workspace context packages

| Capability | Where |
|------------|-------|
| Workspace context export | `GET /workspaces/:id/context` |
| Message edits in thread context | `GET /threads/:id/context` |

## v65.0.0 — App install OAuth

| Capability | Where |
|------------|-------|
| OAuth authorization code | `POST .../apps/:app_id/oauth/authorize` |
| Token exchange | `POST /oauth/app/token` |

## v62.0.0 — Subscribe schema + outbox list

| Capability | Where |
|------------|-------|
| WS subscribe schema version | `subscribe_ack.schema_version` |
| List quarantined outbox | `GET /workspaces/:wid/outbox/quarantined` |

## v60.0.0 — MCP streamable session lifecycle

| Capability | Where |
|------------|-------|
| Streamable session TTL | `MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS` |
| Close streamable session | `DELETE /mcp/streamable` |

## v59.0.0 — Agent integration charter

| Capability | Where |
|------------|-------|
| Agent integration guide | [[Agent Integration]] |
| Event/tool contract CI | `scripts/check-agent-contract.sh` |

## Maidan 2.0 product gate (`maidan-2.0`)

| Capability | Where |
|------------|-------|
| Product Ladder 35–58 closed | [[Retros/Product Ladder 35+]] |
| Checklist sign-off | [[Product Completion Checklist]] at **`v58.0.0`** |

## v58.0.0 — Maidan 2.0 completion gate

| Capability | Where |
|------------|-------|
| Product completion checklist (28–57) | [[Product Completion Checklist]] |
| Expanded completion gate e2e | `product_completion_gate_e2e.rs` |

## v55.0.0 — Helm production bundle

| Capability | Where |
|------------|-------|
| cert-manager ingress values | `helm/maidan/values-cert-manager.yaml` |
| Stack prod bundle | `helm/maidan-stack/values-prod.yaml` |
| kind `helm install` CI | `scripts/helm-install-kind-smoke.sh` |

## v54.0.0 — Capability quotas & distributed limits

| Capability | Where |
|------------|-------|
| Per-token capability quotas | `maidan_token_quotas`, mint `quotas` field |
| Quota enforcement | `maidan-server::quota` middleware |
| Redis rate limiter | `MAIDAN_RATE_LIMIT_REDIS_URL` |

## v53.0.0 — Workspace full erasure

| Capability | Where |
|------------|-------|
| Full workspace delete | `DELETE /workspaces/:id` + `confirm_workspace_id` |
| Deep purge + row delete | `Store::erase_workspace` |
| Pre-delete audit | `workspace.erase` action |

## v52.0.0 — FSM automation hooks

| Capability | Where |
|------------|-------|
| FSM hook CRUD | `POST/GET/DELETE /workspaces/:wid/fsm-hooks` |
| State-filtered dispatch | `maidan-server::fsm_hooks`, `fsm_hook_worker` |
| HTTP + MCP tool handlers | Reuses `SlashHandlerKind` + webhook signing |
| MCP registration tools | `register_fsm_hook`, `list_fsm_hooks` |

## v51.0.0 — Slash commands

| Capability | Where |
|------------|-------|
| `/command` parser | `maidan-router::slash` |
| Slash command CRUD | `POST/GET/DELETE /workspaces/:wid/slash-commands` |
| HTTP + MCP tool handlers | `maidan-server::slash_commands` |
| MCP registration tools | `register_slash_command`, `list_slash_commands` |

## v50.0.0 — Outbound webhooks

| Capability | Where |
|------------|-------|
| Webhook CRUD | `POST/GET/DELETE /workspaces/:wid/webhooks` |
| HMAC-SHA256 delivery | `maidan-server::webhooks` |
| Retry + quarantine queue | `maidan_webhook_deliveries`, `webhook_worker` |
| `EventKind` subscription filters | `maidan-store::webhooks::kinds_match` |

## v49.0.0 — Agent context export

| Capability | Where |
|------------|-------|
| `GET /threads/:id/context` prompt pack | `maidan-server::thread_context` |
| `Store::list_thread_transitions` | `maidan-store` |
| Artifact discovery via message metadata | `thread_context::artifact_shas_from_metadata` |

## v48.0.0 — Search scale & parity

| Capability | Where |
|------------|-------|
| `sqlite-vec` per-connection load + SQL cosine distance | `maidan-search::sqlite_vec`, `SqliteSearch` |
| `SearchHit.score` normalized `[0, 1]` across backends | `maidan-search::hit`, OpenAPI `SearchHit` |
| `maidan_search::sqlite_pool_options()` for vec-enabled pools | `maidan-search`, `maidan-server` SQLite path |
| Scale guidance (Postgres HNSW prod, SQLite dev) | [[Production]], [[Architecture]] |

## v47.0.0 — Per-model embedding tables

| Capability | Surface |
|------------|---------|
| Embedding model registry | `maidan_embedding_models` + `maidan_emb_*` tables |
| Reindex CLI | `maidan reindex-embeddings` |

## v46.0.0 — Edit history & message UX

| Capability | Surface |
|------------|---------|
| Message edit history | `maidan_message_edits`, `GET /messages/:id/edits` |
| UI edited affordance | `/ui` v5 history panel + “edited” on messages |

## v45.0.0 — Admin console

| Capability | Surface |
|------------|---------|
| Operator UI admin | Audit log, purge confirm, federation peers, token revoke |
| Session admin reads | `GET /ui/api/workspaces/:wid/audit`, `.../peers` |

## v44.0.0 — UI collaboration flows

| Capability | Surface |
|------------|---------|
| Operator UI v3 | Thread sidebar, compose/edit, artifact upload, faceted search |
| Session read APIs | `GET /ui/api/channels/:cid/threads`, `.../threads/:tid/messages`, `.../search` |

## v43.0.0 — UI v2 shell

| Capability | Surface |
|------------|---------|
| Operator UI v2 | `/ui` channel sidebar + WS live feed |
| Session channel list | `GET /ui/api/workspaces/:wid/channels` |

## v42.0.0 — Presence & typing

| Capability | Surface |
|------------|---------|
| Ephemeral presence | WS `member_id` + `presence` / `presence_snapshot` frames |
| Typing indicators | WS `{"type":"typing","thread_id",…,"active"}` fan-out |

## v41.0.0 — Reactions & pins

| Capability | Surface |
|------------|---------|
| Emoji reactions | `POST/GET/DELETE /messages/:id/reactions` |
| Thread pins | `POST/GET/DELETE /threads/:id/pins` |
| MCP reactions & pins | `add_reaction`, `remove_reaction`, `list_reactions`, `pin_message`, `unpin_message`, `list_pins` |

## v40.0.0 — Mention router & inbox

| Capability | Surface |
|------------|---------|
| Member inbox + unread cursor | `GET /members/:id/inbox`, `POST /members/:id/inbox/read` |
| `@handle` mention routing | `maidan-router` on HTTP/MCP `post_message` / `post_dm_message` |

## v39.0.0 — Direct messages

| Capability | Surface |
|------------|---------|
| 1:1 DM conversations | `POST/GET /workspaces/:wid/dm`, `POST/GET /dm/:id/messages` |
| MCP DM tools | `open_dm_conversation`, `list_dm_conversations`, `post_dm_message` |
| WS DM filter | `filter.dm_conversation_id` on `/ws/subscribe` and `GET /mcp/stream` |

## v38.0.0 — MCP resource fan-out complete

| Capability | Surface |
|------------|---------|
| Resource notifications on all HTTP mutations | edit, purge, mention, vote + existing tombstone/FSM |

## v37.0.0 — A2A SendStreamingMessage

| Capability | Surface |
|------------|---------|
| A2A streaming task updates | `SendStreamingMessage` on `POST /a2a/v1/rpc` (SSE) |

## v36.0.0 — `mcp-stdio` Postgres

| Capability | Surface |
|------------|---------|
| MCP stdio against Postgres | `maidan mcp-stdio` with `postgres://` `DATABASE_URL` |

## v35.0.0 — MCP streamable bidirectional mux

| Capability | Surface |
|------------|---------|
| Streamable session mux | Follow-up `POST /mcp/streamable` on open `Mcp-Session-Id` → JSON response + SSE push |

## v34.0.0 — MCP streamable session

| Capability | Surface |
|------------|---------|
| Streamable session correlation | `Mcp-Session-Id` on `POST /mcp/streamable` |

## v33.0.0 — MCP resource fan-out (HTTP)

| Capability | Surface |
|------------|---------|
| Resource notifications on tombstone / FSM | HTTP + `GET /mcp/notifications` |

## v32.0.0 — Helm umbrella

| Capability | Surface |
|------------|---------|
| Stack Helm chart (server + optional Postgres/MinIO) | `helm/maidan-stack/` |

## v31.0.0 — Workspace artifact purge

| Capability | Surface |
|------------|---------|
| Purge artifact metadata + blobs | `POST /workspaces/:id/purge` |

## v30.0.0 — HTTP rate limits

| Capability | Surface |
|------------|---------|
| Optional global HTTP rate limit | `MAIDAN_RATE_LIMIT_MAX`, `MAIDAN_RATE_LIMIT_WINDOW_SECS` |

## v29.0.0 — Message edit

| Capability | Surface |
|------------|---------|
| HTTP message edit (body/metadata, `edited_at`) | `PATCH /messages/:id` |
| MCP message edit | `edit_message` tool |
| Bus fan-out on edit | `MessageEdited` event |

## v28.0.0 — Privacy complete

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Deep workspace purge (messages, embeddings, refs, tokens, events) | `POST /workspaces/:id/purge` |
| Workspace-scoped audit list                               | `GET /workspaces/:id/audit`          |

## v27.0.0 — MCP streamable HTTP (Product Ladder close)

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| MCP streamable HTTP subset                              | `POST /mcp/streamable`               |
| Post-ladder backlog register                            | [[Remaining Work]]                   |

Clusters **23–26** in the same release integration ([[Retros/Cluster 23.0]] … [[Retros/Cluster 26.0]]).

## v26.0.0 — Product completion gate

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Product completion checklist                            | [[Product Completion Checklist]]     |
| Completion gate e2e                                     | `product_completion_gate_e2e.rs`     |

## v25.0.0 — Privacy & erasure

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Workspace message purge + audit                         | `POST /workspaces/:id/purge`         |

## v24.0.0 — Deploy & scale (Helm)

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Helm chart (maidan-server)                              | `helm/maidan/`                       |
| Helm template CI smoke                                  | `scripts/helm-template-smoke.sh`     |

## v23.0.0 — Web UI product

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Operator UI: events, search, thread FSM, token mint     | `/ui`                                |

## v22.0.0 — Capabilities hardening

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Documented capability map                               | [[Capability Map]]                   |
| Denial e2e matrix (HTTP, MCP, A2A, WS)                   | `capability_matrix_e2e.rs`           |

## v21.0.0 — A2A agent transport

| Capability                                              | Surface                    |
|---------------------------------------------------------|----------------------------|
| A2A JSON-RPC `SendMessage` / `GetTask`                  | `POST /a2a/v1/rpc`         |
| Outbound A2A client                                     | `maidan-a2a::A2aClient`    |
| Agent card protocol hints                               | `GET /.well-known/maidan.json` |

## v20.0.0 — Message router

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Channel/thread/message hierarchy resolution             | `maidan-router::resolve_*`    |
| HTTP + MCP use shared router                            | `maidan-server`, `maidan-mcp`   |

## v19.0.0 — S3 multipart artifacts

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| S3 multipart upload (begin / part / complete / abort)   | `maidan-artifacts::S3Store`          |
| Multipart artifact HTTP API                             | `/artifacts/multipart`               |
| Multipart artifact MCP tools                          | `begin_artifact_multipart`, etc.     |

## v18.0.0 — SQLite semantic search

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| SQLite embedding storage + semantic search              | `maidan-search::SqliteSearch` |
| HTTP `mode=semantic` on SQLite                          | `GET …/search?mode=semantic`  |

## v17.0.0 — MCP resource fan-out

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Multi-URI fan-out on MCP tool mutations                 | `maidan-mcp::resource_updates` |

## v16.0.0 — MCP HTTP resource notifications

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Shared MCP dispatcher (HTTP)                            | `AppState.mcp`                |
| Resource notification SSE                               | `GET /mcp/notifications`      |
| HTTP + stdio `notifications/resources/updated`          | `maidan-mcp` broadcast        |

## v14.0.0 — SQLite transactional outbox

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| SQLite transactional outbox + relay                     | `maidan-store::sqlite::outbox`, `OutboxRelay` |
| `OutboxBackend` for relay and metrics                     | `maidan-store::outbox`, `AppState` |

## v15.0.0 — MCP resource subscriptions (stdio)

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| MCP `resources/subscribe` / `resources/unsubscribe`    | `maidan-mcp::McpServer`       |
| Resource update notifications on stdio                 | `notifications/resources/updated` |

## v13.0.0 — Delivery contract & subscriber ledger

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Per-consumer delivery cursor (Postgres + SQLite)          | `maidan_delivery_cursor`, `Store::advance_delivery_cursor` |
| Outbox quarantine replay API                              | `POST /workspaces/:wid/outbox/:oid/replay`                   |
| Installed apps + app-scoped tokens                        | `maidan_apps`, `POST /workspaces/:wid/app-installations/:iid/tokens` |
| Optional `consumer_id` on subscribe                       | `/ws/subscribe`, `/mcp/stream` |
| Federation delivery cursor per peer                       | `federation:{peer_id}`        |

## v12.0.0 — Outbox relay hardening

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Outbox quarantine after max relay attempts              | `maidan_outbox.quarantined_at`, `OutboxRelay` |
| `MAIDAN_OUTBOX_MAX_ATTEMPTS`                            | `maidan-server` env           |
| Quarantine / oldest-pending outbox metrics              | `/metrics`                    |

## v11.0.0 — Coverage 11%

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| CI line-coverage floor at 11.0%                          | `.github/workflows/ci.yml`    |
| Outbox/relay/publish deferral test coverage               | `maidan-store`, `maidan-server`, `maidan-bus::test_support` |
| Static UI smoke (`GET /ui/`)                            | `maidan-server/tests/ui_static_e2e` |

## v10.0.0 — Transactional outbox (Postgres)

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Transactional outbox (`maidan_outbox` + relay)          | `maidan-store`, `maidan-server::outbox_relay` |
| Outbox metrics on `/metrics`                            | `maidan_outbox_pending`, `maidan_outbox_relay_total` |
| Outbox ops guidance                                     | [[Production]], [[Architecture]], [[Decisions]] |

## v9.0.0 — Coverage depth

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| CI line-coverage floor at 10.5%                          | `.github/workflows/ci.yml`    |
| Targeted coverage tests (bus, types, server metrics)      | `maidan-bus`, `maidan-types`, `maidan-server` |

## v8.0.0 — Bus hydrate observability

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| `maidan_bus_notify_hydrate_total{result}` on `/metrics` | `maidan-bus::HydrateStats`, `maidan-server::metrics` |
| Bus hydrate alerting and troubleshooting                | [[Production]], [[Operations]], [[Architecture]] |

## v7.0.0 — Bus pointer delivery

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| `Store::get_stored_event(log_id)`                       | `maidan-store::Store`         |
| Postgres NOTIFY `log_id_v1` pointer + hydrate           | `maidan-bus::PostgresBus`     |
| Large event publish beyond legacy NOTIFY JSON cap       | Postgres bus + `maidan_events` |
| Bus pointer delivery ops notes                          | [[Production]], [[Architecture]], [[Decisions]] |

## v6.0.0 — Delivery reliability

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Subscribe lag + replay Prometheus metrics (WS + MCP SSE) | `maidan-server::event_stream`, `/metrics` |
| Indexer age gauge (`maidan_indexer_last_event_age_seconds`) | `/metrics`, `maidan-server::metrics` |
| Postgres listener health/error gauges                   | `maidan-bus::ListenerHealth`, `/metrics` |
| Delivery reliability runbook + alert mapping            | [[Production]], [[Operations]], [[Architecture]] |

## v5.0.0 — Coverage & search quality

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| CI line-coverage floor at 10.0%                         | `.github/workflows/ci.yml`    |
| Optional Codecov upload from CI                         | `codecov/codecov-action`      |
| Model-filtered Postgres semantic search                 | `maidan-search::postgres`, `GET …/search?mode=semantic` |
| `embedding_model` on semantic hits                      | `SearchHit`, OpenAPI          |
| Embedding model/dimension on `/health`                  | `maidan-server::health`       |
| Rank semantics docs (lexical vs semantic)               | [[Architecture]], [[Production]] |

## v4.0.0 — Subscriber continuity

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Signed `resume_token` + `subscribe_ack` (WS + MCP SSE)  | `/ws/subscribe`, `/mcp/stream` |
| `replay_truncated` when replay hits 500 rows            | `maidan-server::event_stream` |
| Subscribe/resume operator docs                          | [[Production]], [[Architecture]], OpenAPI `info.description` |

## v3.0.0 — Search & subscriber depth

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Semantic facets on Postgres (`mode=semantic` + facets) | `GET /workspaces/:wid/search`, MCP `search_messages` |
| WS/MCP auto-replay on bus lag with workspace filter    | `maidan-server::event_stream`, `/ws/subscribe`, `/mcp/stream` |
| CI coverage floor (`llvm-cov --fail-under-lines`)      | `.github/workflows/ci.yml`    |

## v2.1.0 — OIDC operator hardening

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| HMAC-signed session cookie                              | `maidan_session` (`uuid.hmac`) |
| IdP logout redirect                                     | `POST /auth/logout` → `end_session_endpoint` |
| Auth routes in OpenAPI                                  | `/auth/*`, `sessionCookie` scheme |
| Optional auto-mint after login                          | `MAIDAN_OIDC_AUTO_MINT`, `/ui/?auto_mint=1` |
| UI copy-to-clipboard for minted admin secret            | `/ui/`                        |

## v2.0.0 — OIDC identities and human sessions

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| OIDC identity + session persistence (migration 0012)   | `maidan-store`, `maidan-types` |
| OIDC authorization-code + PKCE login flow               | `/auth/oidc/login`, `/auth/oidc/callback` |
| Session cookie + logout                                 | `maidan_session` cookie, `POST /auth/logout` |
| Session introspection                                   | `GET /auth/session`           |
| First-workspace `token:admin` mint via OIDC session     | `POST /auth/session/mint`     |
| Browser UI OIDC sign-in + cookie-backed event tail      | `/ui/`, `/ui/api/workspaces/:wid/events` |
| Mock OIDC for CI (`MAIDAN_OIDC_MOCK=1`)                 | `oidc_e2e.rs`                 |

## v1.4.0 — Auth hardening minor

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Bootstrap routes gated by `MAIDAN_BOOTSTRAP=1` (when auth on) | `maidan-server::bootstrap`, `maidan-server::app` |
| One-shot first-workspace bootstrap enforcement          | `maidan-server::routes`, `maidan-store::Store::count_workspaces` |
| OIDC runtime design spike and phased plan              | `docs/OIDC.md`, `docs/Decisions.md` |

## v1.3.0 — Semantic search UX minor

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Semantic query mode on search (`mode=semantic`)         | `GET /workspaces/:wid/search`, MCP `search_messages` |
| OpenAI-compatible remote embedding provider             | `maidan-search::OpenAiCompatibleProvider`, env config |
| Embedding provider errors surfaced in semantic queries  | `maidan-server::routes`, `maidan-mcp::tools` |
| Embedding indexer failures visible on readiness         | `maidan-server::health`, `EmbeddingHandler` |

## v1.2.0 — Search + embeddings minor

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Pluggable `EmbeddingProvider` (`hash-v1` default)         | `maidan-search`, `MAIDAN_EMBEDDING_PROVIDER` |
| Lexical search facets (`author`, `channel`, `kind`)       | `GET /workspaces/:wid/search`, MCP `search_messages` |
| Postgres `websearch_to_tsquery` operator pass-through     | `maidan-search::query`, Postgres `Search` |

## v1.1.0 — Delivery reliability minor

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Postgres bus listener health on `/health/ready`           | `maidan-bus`, `maidan-server::health` |
| WS/MCP `replay_hint` on bus lag                           | `maidan-server::ws`, `mcp_stream` |
| Resumable subscribe (`after_id`, `Last-Event-Id`)       | `maidan-server::ws`, `event_stream` |
| Encrypted peer outbound secrets at rest                   | `maidan-auth::peer_secret`, migration 0010 |
| `remote_workspace_id` on federation peers                 | migration 0011, `maidan-a2a::Outbound` |
| Federation push + pull compose CI smoke                 | `scripts/federation-*.sh`, `compose.yaml` |

## v1.0.0 — Cluster 1.0 complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Production runbook                                      | `docs/Production.md`          |
| Semver-stable HTTP + MCP API                            | policy in `docs/Decisions.md` |
| `MAIDAN_ENV=production` config guard                    | `maidan-server::config`       |
| Liveness `/health/live` + readiness `/health/ready`     | `maidan-server::health`       |

## v0.7.0 — Cluster H complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Graceful shutdown + `X-Request-Id`                      | `maidan-server`               |
| `/health/live` + `/health/ready`                        | `maidan-server::health`       |
| `maidan mcp-stdio`                                        | `maidan-cli`                  |
| `GET /mcp/stream` (SSE)                                 | `maidan-server::mcp_stream`   |
| Browser UI `/ui/`                                       | `maidan-server/static`        |
| `docs/Production.md`                                    | docs                          |

## v0.6.0 — Cluster G complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Migration 0009 federation peers + ingest dedupe           | `maidan-store`                |
| `FederationEnvelope` / `FederatedEventBatch`              | `maidan-a2a`                  |
| `POST /a2a/v1/events` + peer bearer auth                  | `maidan-server::federation`   |
| `FederationWorker` outbound poll                          | `maidan-server`               |
| Peer CRUD + `/.well-known/maidan.json`                    | `maidan-server`               |
| `federation:ingest` / `federation:admin` capabilities     | `maidan-auth`                 |

## v0.5.0 — Cluster F complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Migration 0008 `maidan_api_tokens`                      | `maidan-store`                |
| `maidan-auth` bearer resolution + capability vocabulary | `maidan-auth`                 |
| HTTP Bearer middleware (`AUTH_DISABLED` for tests)      | `maidan-server::auth`         |
| Per-route capability checks (401/403 problem+json)      | `maidan-server::routes`       |
| WS `SubscribeFrame.token` + `event:subscribe`           | `maidan-server::ws`           |
| MCP `tools/call` / `resources/read` authz               | `maidan-mcp`                  |
| `POST …/members/:mid/tokens` mint (secret once)         | `maidan-server::routes`       |
| `DELETE /tokens/:id` revoke                               | `maidan-server::routes`       |

## v0.4.0 — Cluster E complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| `ArtifactKind` taxonomy + migration 0007                  | `maidan-types`, `maidan-store` |
| `S3Store` + `ARTIFACT_BACKEND=s3`                         | `maidan-artifacts`, compose   |
| `POST /artifacts` + `GET /artifacts/:sha`                 | `maidan-server::routes`       |
| `put_reader` + kind-aware put helpers                     | `maidan-artifacts`            |
| MCP `upload_artifact` + `get_artifact_metadata`           | `maidan-mcp::tools`           |
| MCP `maidan://artifacts/{sha256}` resource                | `maidan-mcp::resources`       |

## v0.3.0 — Cluster D complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Thread FSM + `maidan_thread_transitions` log              | `maidan-fsm`, `maidan-store`  |
| `POST /threads/:id` transitions + 409 on illegal edges    | `maidan-server::routes`       |
| `ThreadStateChanged` event                                | `maidan-types::events`        |
| Nested threads + HSM parent/child rules                   | `maidan-fsm::hsm`             |
| `hash-v1` embedding indexer (Postgres)                    | `maidan-search::EmbeddingHandler` |
| `GET /workspaces/:wid/events` replay API                  | `maidan-server::routes`       |
| MCP `prompts/list` + `prompts/get` (`thread_workflow`)    | `maidan-mcp::prompts`         |

## v0.2.0 — Cluster C complete

| Capability                                                    | Surface                  |
|---------------------------------------------------------------|--------------------------|
| Lexical search (Postgres tsvector + SQLite FTS5)              | `maidan-search::PostgresSearch` / `SqliteSearch` |
| `GET /workspaces/:wid/search` HTTP route                      | `maidan-server::routes`  |
| MCP `search_messages` tool (8th tool)                         | `maidan-mcp::tools`      |
| `<mark>`-wrapped snippet highlights                           | `maidan-search`          |
| `pgvector` semantic search (HNSW cosine, 1024-d)              | `maidan-search::PostgresSearch` |
| `Search::upsert_embedding` / `semantic_search`                | `maidan-search::Search`  |
| Bus-driven background indexer with reconnect backoff          | `maidan-search::Indexer` |
| `EventHandler` trait + `LoggingHandler` baseline              | `maidan-search::indexer` |
| Cross-dialect search parity test                              | `maidan-search/tests`    |

## v0.1.0 — Cluster B complete

| Capability                                                    | Surface                  |
|---------------------------------------------------------------|--------------------------|
| GitHub Actions CI (lint + secrets + test + integration + e2e) | `.github/workflows/`     |
| HTTP CRUD for the core entity set                             | `maidan-server::routes`  |
| RFC 7807 `application/problem+json` error bodies              | `maidan-server::error`   |
| Event taxonomy (`Event`, `EventKind`, `EventFilter`)          | `maidan-types::events`   |
| `InMemoryBus` (tokio broadcast)                               | `maidan-bus::InMemoryBus`|
| `PostgresBus` (LISTEN/NOTIFY, 7990-byte payload cap)          | `maidan-bus::PostgresBus`|
| Every mutation publishes its event                            | `maidan-server::routes`  |
| WebSocket `/ws/subscribe` with filter handshake               | `maidan-server::ws`      |
| MCP `POST /mcp` (initialize + tools + resources)              | `maidan-server::mcp`     |
| 7 MCP tools (list/post/mention/vote/reference)                | `maidan-mcp::tools`      |
| 3 MCP resource URI patterns (workspaces/channels/threads)     | `maidan-mcp::resources`  |
| Cross-arch release binaries (Linux x64/arm64, macOS x64/arm64) on tag push | `.github/workflows/release.yml` |
| Multi-arch ghcr.io image publish on tag                       | `.github/workflows/release.yml` |

## v0.0.1 — Cluster A complete

| Capability                                              | Surface                 |
|---------------------------------------------------------|-------------------------|
| Persistent core schema (Postgres + SQLite)              | `maidan-store`          |
| Dialect detection from `DATABASE_URL` prefix            | `maidan-store::Dialect` |
| Cross-dialect parity test                               | `maidan-store/tests`    |
| Content-addressed artifact body store (LocalFs)         | `maidan-artifacts`      |
| Atomic, dedup-safe artifact writes (50-task concurrent) | `maidan-artifacts`      |
| `/health` endpoint reporting DB + storage status        | `maidan-server`         |
| `docker compose up` brings up Postgres + MinIO + server | `compose.yaml`          |
| Hot-reload dev compose stack                            | `compose.dev.yaml`      |
| Kustomize base + dev + prod overlays                    | `k8s/`                  |
| testcontainers-backed integration suite                 | `maidan-store/tests`    |
| Obsidian docs vault                                     | `docs/`                 |
