# Capabilities

A running list of what Maidan can do, by release. Each cluster's retro
PR prepends a new section so the latest is always at the top.

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
| The falsifiable hello-world: a Python SDK worker + a TypeScript SDK worker both `claim_next_thread` on one channel → Maidan hands each task to exactly one (no cross-language double-claim; drained queue → `null`; no LLM); verified end-to-end via `scripts/lease-demo.sh`. MCP client configs for Cursor/Claude (`/mcp/streamable`, bearer, `2026-07-28`). LangChain/AutoGen examples now **filter to the six-tool hero loop** (client-side; catalog stays 78, pi 8-seam callable) instead of loading all ~78. CI guards the new scripts/configs | `examples/lease_demo/`, `scripts/lease-demo.sh`, `examples/{cursor-mcp,claude-desktop-mcp}.json`, `examples/{langchain,autogen,rest}_maidan.py`, `examples/README.md`, `docs/Framework Integrations.md`, `.github/workflows/ci.yml` |

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
