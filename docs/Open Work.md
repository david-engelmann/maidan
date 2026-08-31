# Open work

Aggregate of deferred items across retros plus standing risks — the
“if I had two hours” backlog. For exhaustive partials and Slack parity,
see [[Remaining Work]].

Updated at each cluster retro. **Baseline:** code on `main` at **`v314.0.0`** (Product Ladder 102+ complete at `v120` / `maidan-scale-1.0`; post-gate hardening 121+; MCP `2026-07-28` 300–303, mail 304–306, Slack/GitHub projectors 307–312, SDKs 294–299, launch-prep 313–314). Reconciled against code at v126 (Cluster 127), v143 (Cluster 144), v273 (Cluster 273), and again at **v314** (2026-08-28 4-thread research sweep — see "Pre-launch fixes + flagship arc" below).

## Post-flagship audit program (2026-08-30 full-repo audit — CURRENT)

A 9-agent full-repo audit (code / deferred / docs / product / perf / security / testing /
architecture → synthesis; journal `wf_23c0c888-03f`) ran after the flagship arc closed at
`v331`. **Verdict: ~90% of "code perfect / docs immaculate / no major gaps / production-ready /
compelling."** Engineering discipline is top-decile (no lib `unwrap`/TODO; REST transactional
outbox; comprehensive app-layer RBAC; LSN causal replica routing validated vs real replication;
an honest self-correcting backlog). **The single dominant theme: the MCP transport was never
brought to parity with REST** — and MCP is the product's primary agent interface, so the
deficient transport is the one agents actually use. The two sharpest items were **code-verified
by the maintainer** (file:line below), not taken on faith. Run as the next program (normal
cluster cadence: retro + `vX.0.0` tag each).

**P0 — fix before promoting:**
- **P0.1 — ✅ FIXED (Cluster 332).** MCP artifact tools now enforce Cluster-204 tenant isolation:
  `get_artifact_metadata` + the `maidan://artifacts/{sha}` resource read gate on
  `artifact_ref_exists(auth.workspace_id, sha)` → `NotFound` when absent (no cross-tenant oracle,
  matching REST); MCP uploads (single-shot + multipart complete) record the per-workspace ref via
  `record_artifact_ref`; `resources::read` uses `meta.size_bytes` instead of loading the blob.
  e2e `mcp_artifact_tools_enforce_tenant_isolation` (workspace B denied on both the tool + resource
  paths; A allowed). *Was:* MCP artifact tools bypassed Cluster-204 (cross-tenant leak).
  `crates/maidan-mcp/src/tools/artifact.rs::get_artifact_metadata` takes no `auth` and calls
  `store.get_artifact_by_sha` with **no `artifact_ref_exists` check**; `crates/maidan-mcp/src/resources.rs:57-60`
  (`maidan://artifacts/{sha}` read) does the same **and returns the full blob bytes** — so any
  `workspace:read` bearer reads any tenant's artifact bytes+metadata by SHA (REST returns 404 via
  `ensure_artifact_ref`). `upload_artifact` also uses `upsert_artifact` (no ref → REST 404). Fix:
  thread `auth` in, gate reads on `artifact_ref_exists(auth.workspace_id, sha)`, upload via
  `upsert_artifact_with_event` + `ref_workspace` (the Cluster-330 `snapshot.rs` tool is the template);
  drop the full-blob read in `resources.rs`, use `meta.size_bytes`. Add a cross-tenant e2e. **Effort S.**

**P1 — high-value clusters (ordered):**
- **P1.1a — ✅ FIXED (Cluster 333):** MCP `edit_message` now uses `edit_message_with_event` + the new
  `McpServer::publish_stored` bus-notify, so an MCP edit appends `MessageEdited` → as-of replay sees the
  edit, the indexer reindexes, WS/SSE + notification router fire. **P1.1b — ✅ FIXED (Cluster 334):**
  the other 7 event-less write tools (`cast_vote`/`add_reaction`/`remove_reaction`/`pin_message`/
  `unpin_message`/`record_mention`/`add_reference`) now use `*_with_event` + `publish_stored`, and MCP
  `post_message`/`post_dm_message` publish `MentionRecorded` per @mentioned member. **P1.1 (MCP
  write-path parity) is COMPLETE** — every MCP mutation emits its domain event like REST (WS/SSE,
  at-least-once, federation, notifications).
- **P1.1 MCP write-path parity: events + atomicity. ✅ VERIFIED (edit_message).** The 8 event-less MCP
  write tools (`cast_vote`/`add_reaction`/`remove_reaction`/`pin_message`/`unpin_message`/`edit_message`/
  `record_mention`/`add_reference`) call plain non-`*_with_event` store methods and append **no** domain
  event; `tools/message.rs::edit_message` calls `store.edit_message` (event-less) → an MCP edit appends
  no `MessageEdited`, so the **flagship as-of replay returns the stale body forever** and embeddings never
  reindex (stale semantic search); MCP `post_message` never publishes `MentionRecorded` (no agent
  `@mention` notifications / `wait_for_mention`). Migrate to `*_with_event` + a shared `McpServer`
  publish; **sequence `edit_message` first** (sharpest correctness bug). **Effort M.**
- **P1.2 — ✅ mostly DONE (Cluster 335).** The user-visible divergence is closed: the MCP context
  assembler now uses batched shared helpers (no per-message N+1) and surfaces an `artifacts` array,
  matching REST; the sha extractor is shared via `maidan_types::artifact_shas_from_metadata`. **Deferred
  (maintainability-only):** the full cross-crate assembler hoist into `maidan-router` — blocked by a
  `ThreadContext` name collision (router already exports a resolution struct of that name) + utoipa-
  feature propagation + a `futures` dep, a multi-cluster refactor whose remaining payoff is only ending
  the `as_of` double-impl (and the trickiest shared logic — the message fold — already goes through
  `maidan_types::reconstruct_messages_through`). Revisit if the two assemblers start to drift.
- **P1.3 — ✅ DONE across both transports (Clusters 336 + 337):** MCP `whoami` tool
  (`{member_id, workspace_id, capabilities, is_bearer, bypass}` from auth) + `initialize.instructions`
  cold-start guide + `AuthContext::capabilities()` (336); REST **`GET /me`** twin
  (`{member_id, workspace_id, capabilities, is_bearer}` from auth, `workspace:read`, full new-route
  preflight) (337). An agent handed only a base URL + token can now self-discover its `member_id` over
  either transport. Optional arg-defaulting (`author_id`/`member_id` ← `auth.member_id`) still deferred
  (touches many tools; self-discovery already unblocks the hero loop).
- **P1.3 (original) Agent cold-start: `whoami` + populated `initialize` instructions.** No `whoami` tool and no
  `/me` route exist, yet every hero-loop tool needs the caller's own `member_id`; MCP `initialize` omits
  the spec `instructions` field. Add a `whoami` tool + `GET /me` (member/workspace/capabilities), populate
  `initialize.instructions` with the 6-tool hero loop, optionally default `author_id`/`member_id` to
  `auth.member_id`. Cheapest adoption unlock. **Effort M.**
- **P1.4 Post-path round-trip reduction.** Split into two clusters.
  - **P1.4a — ✅ DONE (Cluster 338):** `publish_routed_mentions` (REST + MCP) no longer re-runs
    `resolve_message_chain` per post — it routes via `route_mentions_in_message` with the workspace the
    caller already resolved, and short-circuits when the body has no `@handles` (zero store work for a
    plain post). Removed the now-unused `route_mentions_for_message`.
  - **P1.4b — ✅ DONE (Cluster 339):** `maidan_auth::authorize_thread` resolves the thread's
    `ThreadScope {workspace_id, channel_id, thread_id}` **and** authorizes the caller in one fetch;
    `ensure_thread_access` delegates to it (rule single-sourced; also sheds its own duplicate
    `get_channel`). ~30 handlers across `message.rs`/`thread.rs`/`social.rs`/`skills.rs` migrated —
    those using the scope call `authorize_thread`, the rest keep `ensure_thread_access` and drop the
    redundant `resolve_thread_context` + `ensure_workspace`. Behaviour-identical (404/403, same
    messages); thread+channel fetches halve on that surface.
  - **P1.4c — ✅ DONE (Cluster 340):** `maidan_auth::authorize_message` resolves the
    `MessageScope {workspace_id, channel_id, thread_id, message_id}` **and** authorizes in one pass
    (via `authorize_thread`); `ensure_message_access` delegates to it. ~12 handlers in
    `message.rs`/`social.rs` migrated (edit/tombstone/purge/seed use the scope; votes/reactions/
    get/edits/mentions keep `ensure_message_access`). Message-scoped fetches drop ~5→3.
    **Audit P1.4 is complete** (338 + 339 + 340). Residual: the channel-keyed
    `resolve_channel_context` sites (create/list threads) — 2 low-traffic handlers, left as-is.
- **P1.5 Egress wire-path tests + LSN replica CI (already tracked §3.1/§3.2 — now unblocked).** Add a
  base-URL override to `SlackWebClient`/`GithubApiClient` + real-client-against-loopback tests (SMTP vs
  Mailpit); a CI job running `scripts/replica-harness.sh` that un-ignores the LSN routing tests. The two
  genuine test-confidence gaps on shipped launch-narrative features. **Effort M each.**

**P2 — polish (do after P0/P1):** projector link-management REST/MCP surface (only ingress mounted);
A2A gRPC doc contradiction (`Architecture.md` "three transports/gRPC" vs `Protocols.md` "no gRPC binding"
vs ground-truth get/cancel/list only — reconcile to `Claims.md`'s honest "partial"); `Integration.md`
omits the flagship context surface (`as_of`, glossary-in-pack, snapshot, seed); MCP `post_message` skips
slash-dispatch (decide+document); notification-router O(followers) serial round-trips; Store 256-method
god-trait split; `list_threads` unbounded (last unpaginated list); README no visual media / no paste-ready
invite; tool-count drift (`~78`→**84** in `Framework Integrations.md` + `examples/README.md`), README
image pin `v315`→`v331`, `Architecture.md` `Capability-Map.md` dead GitHub link.

**DECLINE / already-covered (not gaps — do not spend a cluster):** the flagship optional tail (seed
`pack`/`prefix`, `WorkSeeded`, `structure_only` template) + Postgres RLS — explicitly DECLINED in
[[Decisions]] (`## Product scope` + `## Security`); the legacy `/inbox` authz "defect" — verified FALSE
POSITIVE (bearer-only, sessions 401); outbox multi-replica double-publish (K8) — deferred with a correct
fix spec; Postgres benchmark numbers / coverage floor / `context_query_count` flake — real but P3.

## Post-272 forward work (next program)

The optional-deferrals sweep (267–272) closed the last program. The next body of
work was scoped by the **2026-08-25 strategy pass** — the detail and rationale live
in the strategy pack ([Handoff.md](Handoff.md) is the index →
[Pre-Public Hardening.md](Pre-Public%20Hardening.md),
[Path to Impressive.md](Path%20to%20Impressive.md),
[Expansion Bets.md](Expansion%20Bets.md), [Launch.md](Launch.md),
[Protocols.md](Protocols.md), [Providers.md](Providers.md)). **This section is the
canonical backlog; the pack is the "why."** Nothing here is committed to as a program
yet — pick the next arc with the maintainer, then run it through the normal cluster
workflow (retro + `vX.0.0` tag each).

| Item | What / why | Detail |
|------|-----------|--------|
| ✅ **MCP `2026-07-28` upgrade** (headline) — **DONE (Clusters 300–303, tags v300–v303)** | The `2026-07-28` revision (stateless Streamable HTTP; `Mcp-Session-Id` + initialize handshake gone; SEP-2243 `Mcp-Method`/`Mcp-Name` routing headers) is shipped: **300** additive negotiation (`SUPPORTED = ["2026-07-28","2024-11-05"]`), **301** stateless streamable core (a 2026 POST lands cold, no session; live-wait on `GET /mcp/stream`/WS), **302** SEP-2243 routing headers (present ⇒ must match body else 400), **303** advertise (default flipped to `2026-07-28`; federation card/reference/Integration/Protocols updated; J2 retired). `2024-11-05` still accepted on explicit request. **Deferred (niche, non-blocking):** stateless server→client (`request_client`) + per-request `_meta.io.modelcontextprotocol/clientInfo`; `ttlMs`/`cacheScope` on list responses; optional `server/discover`. | Protocols.md (J1–J8); Handoff **J3 / M.0** |
| ✅ **Durable mail retry queue — DONE (arc 304–306)** | Email delivery was best-effort, no retry. **304** `maidan_mail_outbox` table + store; **305** router `enqueue_mail` + a `mail_worker` (exp. backoff 30s→1h + dead-letter at 8 attempts, multi-replica-safe); **306** DLQ ops (`GET /operator/mail/dead` + `POST …/requeue`, `token:admin`). **THE DURABLE-MAIL-RETRY ARC (304–306) IS COMPLETE.** Follow-up (non-blocking): retention pruning of terminal outbox rows (the 186 sweeper doesn't cover `maidan_mail_outbox`). | Expansion Bets **Bet 4** |
| ✅ **MCP example pack + hero demo — DONE (Cluster 317, Bet 2 snippet pack)** | Shipped the **two-language lease demo** (`examples/lease_demo/` + `scripts/lease-demo.sh`: Python SDK + TS SDK workers both `claim_next_thread` on one channel → each gets a distinct task, drained queue → `null`; no LLM; verified end-to-end), Cursor/Claude MCP configs (`/mcp/streamable`, bearer, `2026-07-28`), and rewrote the LangChain/AutoGen examples to **filter to the six-tool hero loop** (`claim_next_thread`/`post_message`/`get_thread_context`/`set_thread_result`/`wait_for_result`/`wait_for_ready`). **FILTER ONLY** — the 78-tool catalog is unchanged server-side and the pi 8-method seam stays callable; no `seed_thread_from_message` added. CI guards the new scripts/configs. | Expansion Bets **Bet 2** |
| ✅ **Thin client SDKs — DONE + PUBLISHED (arc 294–299)** | TS/Python/Go/Rust clients under `sdk/` at **0.1.0, LIVE on the registries** (verified 2026-08-28: PyPI `maidan` 0.1.0, npm `maidan` 0.1.0, crates.io `maidan` 0.1.0, `sdk-go-v0.1.0` tag; all four `sdk-release` runs succeeded 2026-08-27). Frozen v1 surface = `docs/Client Contract.md`; interop CI = report-only `sdk-interop` (299). **Remaining (small):** typed response models (0.2) + `sdk/README.md` still says "0.0.1 name-hold" (a lie — folded into the 316 honesty scrub). A second 0.1.0 upload is rejected. | Expansion Bets **Bet 3** |
| **Slack projector** — **IN PROGRESS (arc from 307, config-gated)** | A projector (Slack Events ingress → Maidan thread → streamed egress, **no LLM in Maidan**) — a *projector*, not a product. **307 DONE:** ingress foundation (`POST /integrations/slack/events`, signature-verified + `url_verification`; 404 when unconfigured). **308 DONE:** `maidan_slack_channel_links` (slack channel → Maidan channel/thread/member) + store (link/get/list/unlink) + inbound routing (Slack `message` in a linked channel → Maidan thread; loop-prevention: skips bot/subtype + stamps `metadata.slack`). **309 DONE:** egress — `SlackSender`/`SlackWebClient` (`chat.postMessage`) + `route_message_to_slack` (relays a linked-thread Maidan message to Slack, skips Slack-sourced messages via `metadata.slack`; hooked into the notification-router). **THE BIDIRECTIONAL SLACK PROJECTOR (307–309) IS COMPLETE**, config-gated + loop-safe. Follow-ups (non-blocking): link-management REST/MCP surface (store-level so far), a durable Slack egress outbox (best-effort today), a `thread_id` index on the links table. Live wiring needs David to create a Slack app + set `MAIDAN_SLACK_SIGNING_SECRET`/`MAIDAN_SLACK_BOT_TOKEN`. | Expansion Bets **Bet 1** |
| **Git / forge projector** — **COMPLETE (arc 310–312, config-gated)** | GitHub App webhook → thread → issue/PR comment (GitLab/Gitea later). Explicitly **not** a Copilot clone. **310 DONE:** ingress foundation (`POST /integrations/github/events`, `X-Hub-Signature-256`-verified + `ping`; 404 when unconfigured). **311 DONE:** `maidan_github_issue_links` ((repo, issue)→Maidan channel/thread/member) + store (link/get/by-thread/list/unlink) + inbound `issue_comment`→thread routing (skips `Bot` comments + stamps `metadata.github`). **312 DONE:** egress — `GithubSender`/`GithubApiClient` (REST `POST /repos/{repo}/issues/{n}/comments`) + `route_message_to_github` (relays a linked-thread Maidan message to a GitHub comment, skips GitHub-sourced messages via `metadata.github`; hooked into the notification-router beside the Slack egress). **THE BIDIRECTIONAL GITHUB PROJECTOR (310–312) IS COMPLETE**, config-gated + loop-safe. Follow-ups (non-blocking): the full GitHub App JWT/installation-token auto-exchange + Check Runs (a configured PAT/installation token works today), link-management REST/MCP surface (store-level so far), a durable egress outbox (best-effort today). Live wiring needs David to create a GitHub App + set `MAIDAN_GITHUB_WEBHOOK_SECRET`/`MAIDAN_GITHUB_TOKEN`. | Expansion Bets **Bet 6** |
| **Pre-public cleanup nits → superseded by Clusters 315 (correctness) + 316 (honesty scrub)** | The real, verified nits (mail.rs "Not wired", outbox `FOR UPDATE SKIP LOCKED`, `event_stream` swallowed cursor, and the full doc-lie list) are now itemized under "Pre-launch fixes + flagship arc" below. This row is retired into those two clusters. | Pre-Public Hardening.md (**A–K**) |
| **Provider recipes** | Doc/compose recipes only (Ollama/TEI embeddings, R2/AWS-S3 next to MinIO, Keycloak + a SaaS OIDC, Neon/RDS/Supabase note, LibSQL/Turso feasibility). | Providers.md (**I2–I6**) |
| **Public launch** | Public-preview cut, un-hold, announce — **gated on the maintainer's explicit go**; keeps `publish = false` (no crates.io 1.0). | Launch.md (**L1–L6**) |

## Pre-launch fixes + flagship arc (2026-08-28 research sweep)

A 4-thread research sweep (2026-08-28) audited the tree at **v314** for anything more pressing
than the docs scrub, and researched the primitives that make Maidan exceptional at being *the
room*. Folded here. **Lane tags:** `generic-room` (any waiter, incl. pi), `oss-adoption`
(stars/`/play`/registries), `first-consumer` (pi+soundcheck+bgv3 — lives in **pi**, not a maidan
cluster; pi SR-1). **Sequence:** correctness first (315) → honesty/no-clone (316) → snippet pack
(317) → token evidence (318) → the fidelity+context flagship arc. Full rationale: the strategy
doc `docs/Undeniable Final.md` + the four sweep reports (verdicts in-line here are the canonical
fold; that doc is the "why").

### Cluster 315 — pre-launch correctness & security (`generic-room`)

Small, verified code fixes. Cleared as NOT findings (stale docs narration, verified fixed in code):
the `subscribe_grants` self-assertion, the DM generic-route participant gap, and single-tx
dual-write atomicity are all **closed** — see the standing-risks corrections below.

- **legacy `/members/:id/mentions` + `/inbox` self-only — ✅ DONE, but the "live defect" was a
  FALSE POSITIVE on verification.** The audit flagged these (`routes/member.rs`) as missing
  `ensure_acting_member` → "a session can read another member's inbox." **On verification the routes
  are mounted ONLY on the bearer-only `protected` router (`auth::middleware`, no session cookie
  accepted → a session gets `401`); there is no `/ui/api` mount** (unlike the notification handlers,
  which Cluster 251 *did* session-mount — that's why *their* guard is load-bearing). The only callers
  are bearers, which are **act-as-any by design** (the 202/203 model). So there is no
  session-exploitable gap. Still added the three `ensure_acting_member` guards as **defensive
  consistency** (strict no-op for current callers; pins a session to self IF these are ever
  `/ui/api`-mounted like 251). Test: `legacy_inbox_and_mentions_are_bearer_only_not_session_reachable`
  (documents the 401 reachability truth); guard logic unit-tested in `ensure_acting_member`.
- **`hash-v1` embedding default boots with no warning** (`main.rs:247-251`) — "semantic search"
  silently returns near-random results if `MAIDAN_EMBEDDING_PROVIDER` is unset. `warn!` at boot.
  (Repo's own K5.)
- **`event_stream` replay swallows the cursor advance** (`event_stream.rs:202-204`, `let _ =`) — a
  failed advance is invisible (correctness is safe — a stuck cursor re-delivers, never skips; only
  observability suffers). Log/count it. (Repo's own K9.)
- **README "Run it (SQLite, no Docker)" 28-byte secret** (`README.md:149`,
  `MAIDAN_SESSION_SECRET=dev-session-secret-change-me`) won't boot — `session/cookie.rs:18` needs
  ≥32 bytes. Fix to a ≥32-byte value (the 314 headline one-liner was fixed; this sibling was missed).
- Optional defense-in-depth (K3/K4): `AppState::subscribe_resume_secret()` getter `panic!`
  (`state.rs:307`) → boot invariant; gate the `AUTH_DISABLED` + missing-secret test-secret fallback
  (`main.rs:361-368`) behind an explicit `MAIDAN_ALLOW_INSECURE_RESUME_SECRET=1` ack. *(Deferred from 315 — behaviour-changing, low urgency.)*
- **DEFERRED from 315 to its own cluster — outbox `list_pending` `FOR UPDATE SKIP LOCKED`** (K8,
  `postgres/outbox.rs:29-50`): two relay replicas can both fetch + publish the same pending row
  before either `mark_published`. Bounded (optimistic bus is at-most-once, consumers idempotent by
  `log_id`). **A naive `FOR UPDATE SKIP LOCKED` on the pooled SELECT is a no-op false fix** — the
  lock releases when the statement's implicit tx ends, and the relay publishes + marks *outside* any
  tx. A correct fix needs either a **lease column** (migration + the mail_outbox/scheduler pattern)
  or wrapping the batch publish inside a held transaction (holds row locks across bus publishes —
  a robustness trade-off) + a multi-replica double-publish integration test. Its own small cluster,
  not a hasty 315 line.

### Cluster 316 — honesty scrub + no-clone image (docs, `oss-adoption`) — ✅ DONE

- **Docs honesty scrub — DONE.** Corrected `Claims.md` A2A-gRPC overclaim (gRPC = task
  read/cancel/list only, no `SendMessage`); `mail.rs` "Not wired" comment (wired 249; unchecked
  Pre-Public-Hardening A6/K1); `mcp/server.rs` default-2024 comment (const is 2026); `Framework
  Integrations.md` 2024→2026; **two more won't-boot commands fixed** — `Pi.md`'s
  `docker run -e AUTH_DISABLED=1` (missing the ack → fail-closed; + `:latest`→`:v315`, + a
  `maidan init` auth-on path) and `book/src/introduction.md`'s `cargo run` (no `MAIDAN_SESSION_SECRET`,
  same class as the README headline); `Threat-Model.md` seed → `maidan init`; `sdk/README.md` +
  `Clients.md`/`Client Testing.md` banners (0.1.0 published, not name-holds; MCP 2026); `Promotion.md`
  state banner (projectors/mail/SDK shipped, topics set, hero no longer cargo+AUTH_DISABLED); README
  "experimental A2A bridge"→"A2A v1.0 (JSON-RPC+REST; gRPC partial)"; `AGENTS.md`/`Integration.md`
  MCP-2024/A2A-subset; `CLAUDE.md` latest-tag v273→v315 + "Open Work is canonical"; `SECURITY.md`
  cosign example → `<tag>`.
- **No-clone image — smoke-gated, and the smoke reshaped it (KEY FINDING).** The published
  `ghcr.io/david-engelmann/maidan-server:v315.0.0` **boots with auth on** (verified: `/health` ok,
  multi-arch amd64+arm64, anonymously pullable), BUT it is **distroless (no shell) and bundles only
  `maidan-server`, not the `maidan` CLI**, and `POST /workspaces`→401 — so the doc's planned
  "`docker run …` then `exec maidan init`" is **impossible** (no CLI, no shell, no out-of-the-box
  token seed). Added an **honest** README "Prebuilt image (no clone)" note instead: images are signed
  + multi-arch, seed via `maidan init` run against your DB (release binary / one-shot job), verify
  cosign. **Deferred to its own cluster:** a *true* one-command no-clone eval (with the token flow
  bundled) needs the **quickstart image** (both binaries + a shell) **published to GHCR** — real infra,
  not a docs line.
- **Housekeeping — DONE:** `v300.0.0` GitHub Release was a stuck Draft → **published** (neighbors were
  all published). **No `v311` tag** (311's code is in `v312`, commit `6d3172c`) — documented, tag NOT
  cut (correct). Quickstart image pin bump v312→v315 deferred (cosmetic).
- **Residual (fuller pass, low blast radius):** the dense planning docs (`Clients.md`, `Client
  Testing.md`, `Promotion.md`) still have inline 0.0.1/2024/AUTH_DISABLED references beyond the
  corrected top banners; the strategy pack (Handoff/Path/Expansion Bets/Launch/Adoption) stays a
  frozen 2026-08-25 snapshot (canonical = Open Work). Maintainer-facing; a full sweep is optional.

### Cluster 318 — token-pack evidence (`generic-room`) — ✅ DONE

Shipped `token_pack` (`crates/maidan-server/tests/token_pack.rs`, the `load_baseline` pattern:
`#[ignore]`d harness + pure estimator unit-tested in CI) — the scoped context pack vs dumping the
whole channel = **~6.8× fewer tokens** (in-process SQLite, 8 threads × 40 msgs; scoped ~4 951 vs
naive ~33 908 tokens), plus ~1.3× from lean edits. Bytes exact, `≈chars/4` tokens, ratio
tokenizer-independent. `Benchmark.md` "Context-pack token savings" section + `Claims.md` token row →
"Shipped + measured" with the evidence link — a ratio now exists behind the claim.
**Follow-ups (optional):** measure the MCP `get_thread_context` pack separately (it omits artifacts
vs REST); a `maidan_context_tokens_total` metric (not needed now — the doc exists). **This closes the
launch-prep leg of the sweep (315–318); next is the fidelity + context flagship arc.**

### Fidelity + context flagship arc (`generic-room` — the differentiator)

> **✅ COMPLETE (Clusters 319–331, tags `v319.0.0`–`v331.0.0`).** Typed relations (319–320) →
> glossary store/REST-MCP/context-fold (321–323) → vote confidence (324) → agent conventions
> (325) → as-of context replay (326) → seed-from-message REST+MCP (327–328) → immutable context
> snapshot artifact REST+MCP (329–330) → arc closeout, optional tail declined (331). The optional
> tail (seed `pack`/`prefix` inclusion, `WorkSeeded`, flow template) is **declined**, composable
> from shipped primitives — see the ADR
> [[Decisions]] "Product scope". Items 1–7 below
> are the original plan, each annotated with its shipped/declined status.

From two converging research threads (pre-LLM annotation tooling + grounding/argumentation/provenance
theory) plus the context/replay thread. This is the promotable category nobody ships: **the room
where agents build durable, checkable, replayable shared understanding — at a fraction of the
tokens.** It is **one arc on one substrate** (the typed reference edge). All rows are storage+API
level — the server stores and serves typed edges, definitions, and immutable snapshots; **agents
interpret and re-run. Maidan stays a room, not a brain.** Sequence measure-cheap-first (each a
foundation for the next); zero-blast-radius foundations follow the 159/217/234 pattern.

1. **Typed reference relations (keystone) — ✅ DONE (Cluster 319).** `Reference.relation` is now a
   controlled `RelationKind` (`supports / refutes / defines / depends / duplicates / grounds /
   supersedes` + `Other(String)` escape so expressivity isn't lost), not a free string. Serializes as
   the bare snake_case string (wire byte-identical); both store backends bind `as_str()`/parse
   `from_wire` (column stays TEXT, no migration); REST `CreateReference` + MCP `add_reference` inputs
   typed; `ReferenceAdded` carries it; OpenAPI/MCP schemas unchanged (`string`). The thread-DAG was
   already a special-cased typed `blocks`. **Reverse-edge / by-type queries — ✅ DONE (Cluster 320):**
   `Store::list_references_to` (reverse, reuses `idx_references_dst`, no migration) + `GET /references`
   reshaped to query src-or-dst + optional `relation` filter + a new MCP `list_references` tool — "what
   refutes X / what references this" is now navigable. The "vocabulary registry" framing folds into the
   glossary (item 2); `RelationKind::CONTROLLED` is the controlled set for relations.
2. **Shared glossary / definitions layer — foundation ✅ DONE (Cluster 321).** One **flat**
   `maidan_glossary_terms {id, workspace_id, term, definition, aliases, created_by, created_at,
   updated_at}` table (pg `0053` / sqlite `0052`, `UNIQUE(workspace_id, term)`, aliases as JSONB /
   TEXT-JSON) + `GlossaryTerm`/`NewGlossaryTerm` models + `Store::{set,get,list,delete}_glossary_term`
   (both backends; `set` upserts). Workspace-scoped (dropped the speculative `channel_id?`). Kept
   **flat — no hierarchy/broader-narrower** (that's the KG-product line, a locked anti-goal). The
   `defines` edge's target; the anti-drift pin. **REST + MCP CRUD — ✅ DONE (Cluster 322):**
   `PUT/GET/DELETE /workspaces/:wid/glossary/:term` + list, and MCP `set/get/list_glossary_term(s)`
   (`delete` REST-only). **Context-pack fold — ✅ DONE (Cluster 323):** `GET /threads/:id/context` +
   `GET /workspaces/:wid/context` (REST + MCP) carry a `glossary` field (`include_glossary`, default
   `true`, empty-omitted; workspace pack carries it once at the top, deduped). **The glossary layer
   (321→322→323) is COMPLETE.**
3. **Optional `confidence` — ✅ DONE for `Vote` (Cluster 324):** nullable `maidan_votes.confidence`
   (pg `0054` / sqlite `0053`), `Vote`/`NewVote` `Option<f64>` (omitted when absent), REST
   `POST/GET /messages/:id/votes` + MCP `cast_vote`, range `0..=1` at the API edge, re-cast upserts
   it. (`ThreadResult` already stores arbitrary JSON, so a `confidence` there is a convention, not a
   schema change — folded into the conventions.) **Conventions — ✅ DONE (Cluster 325):** documented in
   `docs/Integration.md` "Agent conventions" with a convention-proving `decision_convention_e2e` and
   **zero new server code** — a decision-record shape (kind/status/context/decision/consequences/
   alternatives) over `thread_results`, supersession via the `supersedes` reference edge + `status`
   flip, and an `ack` grounding vote (version-pinned by time: stale once the message is edited after
   the ack's `created_at`; optional `confidence`). **Item 3 (confidence + conventions) is COMPLETE.**
4. **As-of context replay — ✅ DONE (Cluster 326).** `GET /threads/:id/context?as_of=<event_id>`
   + MCP `get_thread_context` `as_of` arg reconstructs a thread as it stood at that event-log id,
   **deterministic over the immutable log** (no fresh search). `Store::list_thread_events_through`
   (both backends) + shared `maidan_types::reconstruct_messages_through` fold `MessagePosted`/
   `MessageEdited` (full `Message` payloads) + `MessageTombstoned` → the as-of message set with
   as-of bodies (a since-edited message shows its old body, a since-tombstoned message reappears);
   additive components cut by the anchor's time; glossary omitted; unknown id → `404`. Serves audit +
   re-ask-from-before-a-tangent. **Deferred:** workspace-context as-of (thread-scoped only in v1).
5. **Seed-from-message gesture** (the write side of "re-ask"). **REST — ✅ DONE (Cluster 327):**
   `POST /messages/{id}/seed` (`{title, inclusion?, channel_id?}`) spawns a titled, claimable child
   thread + a `seeded_from` reference edge (new thread → source), source untouched, N per source,
   gated `workspace:write` + source read + target-channel write. Inclusion `pointer` (default) +
   `quote` shipped; **lineage is the `seeded_from` typed edge (from #1), NOT a bespoke table** — no
   new event kind (emits `ThreadCreated` + `ReferenceAdded`). New `RelationKind::SeededFrom`.
   **MCP `seed_from_message` — ✅ DONE (Cluster 328)** (twin of the REST route; atomic `*_with_event`
   + bus-notify; 83 tools). Seed-from-message is COMPLETE over REST + MCP (pointer + quote).
   **`pack`/`prefix` inclusion + `WorkSeeded` — DECLINED (Cluster 331,
   [[Decisions]] "Product scope"):** composable
   today as snapshot (#6) + seed-pointer + as-of replay (#4); `WorkSeeded` is covered by
   `ThreadCreated` + `ReferenceAdded`. Revisitable with demand.
6. **Immutable context snapshot artifact — ✅ DONE (Cluster 329).** `POST /threads/:id/context/snapshot`
   freezes the assembled pack (live or `as_of`) into the existing content-addressed artifact store
   (sha256 dedup, ref-guarded per 204); returns the `Artifact` (`kind=context_snapshot`), fetchable at
   `GET /artifacts/:sha`; gated `artifact:upload` + thread access. New `ArtifactKind::ContextSnapshot` +
   migration pg `0055` / sqlite `0054` widening the kind `CHECK`. Delivers tamper-evident "exactly what
   the agent was handed" + "prefix paid once, N angles". **MCP `snapshot_thread_context` — ✅ DONE
   (Cluster 330)** (twin of the REST route; modern `upsert_artifact_with_event` + Cluster-204 ref +
   bus-notify; 84 tools). Context snapshot is COMPLETE over REST + MCP. **Remaining (optional
   convenience):** the seed `pack` inclusion (attach a snapshot sha, ties #5↔#6 — composable today
   as snapshot + seed-pointer).
7. **Flow / setup template — DECLINED (Cluster 331,
   [[Decisions]] "Product scope").** Cloning a
   setup (channels/skills/schedules/DAG skeleton) is covered by workspace export (187) + import-remap
   (269–270): export, prune content, import. A dedicated `structure_only` export filter is the arc's
   highest-scope-creep item and "the room never scores which template is better" (a locked
   anti-goal); declined until a research round shows concrete demand.

**Anti-goals (LOCKED — this is what keeps it "perfect at what it does, not more"):** no span-labeling
UI, no inter-annotator-agreement metrics / adjudication queues, no Snorkel-style label model, no
coreference equivalence classes, no rich claim/argument graph (SciClaim), no bespoke decision-record
subsystem, no notes layer (a note is already a message + a reference edge), no KG hierarchy; no server
re-execution of models/tools, no branch-tree-with-merge, no A/B-eval over templates/contexts, no
prompt/version registry, no deep-copy fork, no non-deterministic "improved" replay, no shared-mutable
working set across forks. **Not a harness. Not a labeling product. Not a reasoning engine. Not a
SaaS.** The parked V-track (V1–V8 in `docs/Undeniable.md` §5), the V2 working-set-budget, `/play`,
hosted cloud, and the public launch stay **gated on David**.

### Integration reality — projector/transport test coverage (`generic-room`)

From the 2026-08-29 mocks-vs-e2e audit (`docs/Integration Reality.md`, line-checked). The
in-process room e2es are real; the **vendor-shaped HTTP paths are not exercised** — the shipped
`SlackWebClient`/`GithubApiClient`/`lettre SmtpTransport` are never constructed in any test (only
the `SlackSender`/`GithubSender`/`MailTransport` **trait mocks**, which prove loop-prevention, not
the wire). Both egress clients also **hardcode the vendor host** (`slack.com`, `api.github.com`)
with **no base-URL override**, so they can't be aimed at a local sink. Fold as `generic-room`
test-confidence work (not next; the flagship arc leads):

- **Real-client HTTP-path tests, no SaaS (Integration Reality §3.1).** Add a **base-URL override**
  to `SlackWebClient` + `GithubApiClient`, then a second test that constructs the *real* client
  against a loopback axum sink — assert the JSON body, bearer/`User-Agent`/`Accept`, and the
  failure branches untested today (Slack's HTTP-200 `{"ok":false}`; GitHub's non-2xx status). Copy
  the `webhooks_e2e.rs` loopback-`/hook` shape; for SMTP, drive `SmtpTransport::send` against
  **Mailpit** in Docker. The trait mocks stay for loop-prevention. Don't add a WireMock crate if an
  axum sink is simpler; never hit slack.com / api.github.com in PR CI.
- **LSN replica routing is claimed but CI-untested (§3.2).** The read-your-writes contract
  (`Maidan-Consistency-Token`) has no running-server CI coverage — the harness tests
  (`replication.rs`/`read_routing.rs`/`replica_routing.rs`) are `#[ignore]`d and `MAIDAN_DB_REPLICA_URL`
  is unset in `ci.yml`/`compose.yaml`. Fold: a compose primary+standby stand-in (or a job that runs
  `scripts/replica-harness.sh` + un-ignores those tests). **Distinguish the two env names** — the
  harness reads `MAIDAN_PRIMARY_URL`/`MAIDAN_REPLICA_URL`; the *server* reads `MAIDAN_DB_REPLICA_URL`
  (whether the running server actually routes given that key is a separate, currently-untested check).
- **Honesty nits (§3.4, small docs/naming):** the `*_e2e.rs` files run in the **`integration`** job,
  while the job named **`e2e`** is docker-compose smoke (Operations/Client Testing wording);
  `slack_egress_e2e`/`github_egress_e2e`/`mail_worker_e2e` **overclaim** (trait mocks, not wire
  e2e); `two_replica_*_e2e` is two app sides on one Postgres (app-HA), not `MAIDAN_DB_REPLICA_URL`
  replica routing; `Client Testing.md` still frames SDK CI as `compose --profile full` when the
  real `sdk-interop` job is report-only SQLite + `AUTH_DISABLED`.
- **Live projectors = David's setup, NOT a maidan cluster (§3.3):** a throwaway Slack workspace +
  app and a GitHub App on a throwaway repo behind a tunnel, one scripted round-trip each
  (loop-prevention the assert), nightly/manual. Do NOT start the live apps until §3.1 can aim the
  clients at a local host (else the first live run is also the first HTTP run).
- **Anti-goals (§4):** do NOT rebuild the room e2es (claim/MCP/auth/outbox are real); no Slack
  Marketplace / Check Runs / Copilot; no real tokens in CI; do NOT graduate `sdk-interop`/`a2a-interop`
  off report-only here; do NOT boot `compose.quickstart` in CI as the projector sink; compose
  federation-pull is **not** missing (`federation-pull-smoke.sh` covers it); not a harness.

### Public-launch readiness (external review, 2026-08-25)

An independent agent review ran the released `v272.0.0` binary and audited the repo
for public-launch readiness. Verdict: the core is strong (it independently praised the
self-healing NOTIFY floor, workspace-sharded fan-out, LSN causal replica routing, and
typed IDs — see the "code-backed talking points" below), and the blockers are
onboarding, honesty, and evidence — not missing features. Verified findings, folded
here as the canonical backlog:

| Pri | Item | Evidence / why | Notes |
|-----|------|----------------|-------|
| ✅ **Done (276)** | **Runtime version was `0.0.0`** | `/health` reported `0.0.0` because the release pipeline never set `MAIDAN_VERSION` (the `version()` override already existed). Fixed: the release build bakes the tag into the binary (native + cross via `Cross.toml` passthrough) and the image (`Dockerfile` `ARG`/`ENV`), with a `build.rs` `rerun-if-env-changed` so a warm cache can't ship a stale version. Cargo `version = "0.0.0"` intentionally stays (workspace is `publish = false`). **Follow-up:** an automated release-time assertion that binary/health/image-label/tag agree (currently self-proven by the release run) |
| ✅ **Done (277)** | **SQLite `database is locked` under write contention** | Root-caused: SQLite is single-writer and sqlx's `pool.begin()` is *deferred*, so a multi-connection pool lets two writers each take a read snapshot and race to upgrade — a genuine deadlock `busy_timeout` can't resolve (a contention test showed a warm 8-connection pool failing ~90% of read-modify-write txs; 1 connection is clean). Fixed: the SQLite backend defaults to **one connection** (`maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS`, overridable via `MAIDAN_DB_MAX_CONNECTIONS`); Postgres unaffected. Guarded by `sqlite_write_contention` (default is clean under contention; an `#[ignore]`d probe documents the multi-connection deadlock). **Follow-up:** a read-pool + single-writer split (or `BEGIN IMMEDIATE` writes) would restore SQLite read concurrency without the deadlock, if it ever matters for the single-node backend |
| ✅ **Done (278)** | **One-command quickstart** | `docker compose up` started only Postgres and the `full` profile built from source — no 5-minute path. Shipped `compose.quickstart.yaml` + `docker/Dockerfile.quickstart` (pulls a **pinned, SHA-verified** `v277.0.0` release binary; SQLite + localfs + loopback + the `MAIDAN_ALLOW_INSECURE_NO_AUTH` ack; runs non-root) + `scripts/quickstart-two-agents.sh`. **Built + run end-to-end locally** (image builds, `/health` reports `v277.0.0`, no SQLite lock thanks to 277, the two-agent demo passes). CI guards the files' validity (compose config + `bash -n`) in the compose-smoke job. **Follow-up:** a full run-the-demo CI smoke (source-built server on SQLite/no-auth) — deferred to avoid a network/distroless-perms-flaky job |
| ✅ **Done (279)** | **`maidan init` for clean bootstrap** | Prod image is `--no-default-features` (bootstrap routes stripped) → "need an admin token to create the first admin token". Shipped `maidan init`: connects + migrates, creates the first workspace + admin member (via the `*_with_event` store methods) + mints an **all-capabilities** token (new `capability::all()`), prints the secret once to stdout, and **refuses if the store already has a workspace**. Removes the need for public bootstrap HTTP routes or `AUTH_DISABLED` in production. Documented in Production.md; guarded by a `maidan-cli` integration test (bootstrap-once / refuse-twice) |
| ✅ **Done (arc 282–289)** | **A2A v1.0 compliance** | The A2A endpoint was an experimental Maidan subset. User chose the **full multi-transport + TCK** scope. Grounding in the authoritative spec (`a2aproject/A2A` `a2a.proto` + §5.3 mapping) corrected the backlog's premise: the JSON-RPC method strings are the canonical operation names (`SendMessage`, not `message/send`), the `TASK_STATE_*` enum already conforms, and an Agent Card already exists — so the real gaps are narrower than "everything" | **282 ✅** JSON-RPC method names to spec (`CancelTask`, `{Create,Get}TaskPushNotificationConfig`; dropped non-spec `tasks/resubscribe`). **283 ✅** `ListTasks` (RBAC-filtered) + `GetExtendedAgentCard`. **284 ✅** per-task push-config model + all four push-config ops (`Create`/`Get`/`List`/`Delete`). **285 ✅** Agent Card §4.4.1 schema (supportedInterfaces + capabilities/skills/provider/modes). **286 ✅** HTTP+JSON/REST binding (§11): 9 request/response routes under `/a2a/v1`. **287 ✅** gRPC binding (§10): tonic `A2AService` on a config-gated port, vendored codegen. **288 ✅** transport negotiation (§5.2): configurable absolute-URL + gRPC `AgentInterface` entries. **289 ✅** interop conformance client (`examples/a2a_interop.py`) + harness + report-only CI job; live-verified. **ARC COMPLETE.** Follow-ups (logged, non-blocking): gRPC `SendMessage`/push/streaming, streaming REST endpoints, full A2A error-taxonomy alignment, Helm/compose gRPC port, Agent Card optional fields (securitySchemes/signatures/iconUrl), an official a2a-sdk/TCK-based CI (vs the hand-written conformance client). Deferred within-arc: gRPC SendMessage/push/streaming/extended-card + Agent Card gRPC interface (→288), streaming REST endpoints, A2A error-taxonomy alignment, old workspace-level push table cleanup, push-config `token`/`authentication` fields, list pagination, Agent Card optional fields, absolute interface URLs. Plan in scratchpad `a2a-v1-arc-plan.md` |
| ✅ **Done (280)** | **LangChain + AutoGen recipes** | Shipped copy-paste, **live-verified** recipes: `examples/{langchain,autogen,rest}_maidan.py` + `docs/Framework Integrations.md`. LangChain (`MultiServerMCPClient`) and AutoGen (`StreamableHttpServerParams` + `mcp_server_tools`) each load all 78 tools against a running Maidan. Baked in the `mcp>=1.9,<2` pin (SDK 2.x drops modules the adapters import) and fixed the one untyped catalog param (`set_thread_result.result`) so AutoGen's strict converter accepts the whole catalog. **Follow-up (P2):** a required interop CI job (init → list tools → one read → one write → denied-channel) — deferred as network/adapter-version-fragile; the guide's "Keeping these honest" section prescribes manual re-verification before a pin bump |
| ✅ **Done (281)** | **Published benchmark methodology** | Shipped `docs/Benchmark.md` (published) + a `post_to_observer_latency` measurement in the loadgen harness. Reproducible numbers on named hardware/commit/backend: Apple M3 Max / in-process SQLite (one connection) → post→observer p50 0.71 ms/p99 1.00 ms; mixed throughput 1 586 ops/s (8 workers) / 666 ops/s (32, the single-writer SQLite ceiling), zero errors. Also fixed the harness to benchmark the shipped 1-connection SQLite default (was 16 → the Cluster-277 deadlock). **Follow-ups:** a first-class in-harness Postgres testcontainer target (multi-writer numbers beside SQLite; benchmark-able today via `MAIDAN_LOADGEN_URL` against a running Postgres deployment); a real embedding-provider latency axis |
| ✅ **Done (292)** | **Architecture docs currency + split** | Split into a current, version-neutral conceptual `Architecture.md` + `Architecture-history.md` (release-by-release record). The conceptual doc was also refreshed for currency (it had gone stale ~v104 — the agentic task layer, notifications, three-transport A2A, LSN read-replica, and per-channel RBAC now described); no vX.0.0/cluster vocab on the first user-facing page |
| ✅ **Done (293)** | **GitHub metadata + repo polish** | Set the repo homepage (published docs site) + 10 topics (rust, multi-agent, mcp, model-context-protocol, a2a, ai-agents, agent-infrastructure, agentic, postgres, websocket) via `gh`; added `.github/ISSUE_TEMPLATE/` (bug / protocol-compat / benchmark + config). **Follow-up:** a terminal GIF / screenshot for the README + repo card (needs a recorded asset) |
| ✅ **Done (313)** — L1 / F4 | **Default-secure quickstart** | The quickstart taught `AUTH_DISABLED` as the happy path ("one AUTH_DISABLED screenshot kills the launch"). Now `compose.quickstart.yaml` runs auth ON (dev `MAIDAN_SESSION_SECRET` + `MAIDAN_BOOTSTRAP=1`); the README mints a bearer token via `maidan init` and runs the two-agent demo with it; `scripts/quickstart-two-agents.sh` is auth-aware (`MAIDAN_TOKEN`/`MAIDAN_WORKSPACE`). `AUTH_DISABLED` demoted to a labelled local-only appendix (`compose.quickstart.insecure.yaml`). Quickstart image bumped `v277.0.0`→`v312.0.0` (re-pinned tarball SHAs; `maidan init` landed `v279`). Both paths validated end-to-end; CI validates both compose files |
| ✅ **Done (314)** — L3 / L4 / L5 / L6 | **Launch honesty: claims sheet, policies, release verification** | Writing the claims sheet caught a real bug — the README headline one-liner didn't boot (auth on needs a ≥32-byte `MAIDAN_SESSION_SECRET`); fixed + verified. Shipped `docs/Claims.md` (published, README-linked) mapping every claim → gate/test/"not yet"; a keyless-cosign "Verifying a release" section in `SECURITY.md`; `CHANGELOG-highlights.md` + a Release-notes template; and reconciled `CONTRIBUTING.md` to the solo-maintained/admin-merge model. **All launch-prep is done (313 F4 + 314 L3–L6). The public launch itself stays gated on the maintainer's explicit go (Launch.md).** |

**Code-backed talking points the review validated** (use for the launch narrative — all
shipped, honest): the self-healing Postgres NOTIFY floor (pointer signal + durable log +
gap backfill, Cluster 258), workspace-sharded Tokio fan-out (Cluster 201), LSN
causality-token replica reads (Clusters 261–266), and typed non-interchangeable IDs +
SQLite/Postgres `Store` parity. Positioning moved off "Slack for agents" to the durable
shared-workspace framing (Cluster 274).

### Adoption & ecosystem (deferred / post-launch)

Folded here from the concurrent agent's adoption/SDK strategy pack (Cluster 291,
2026-08-27) so Open Work stays the single backlog source. The detailed specs live in
`docs/Adoption.md` (the funnel + hosted playground/cloud + client program), `docs/Clients.md`
(SDK implementation plan), `docs/Client Contract.md` (the frozen v1 SDK surface), and
`docs/Client Testing.md` (black-box scenarios that double as server coverage) — those are
the spec/index behind these items, not a competing backlog. **All gated: none of this
starts without an explicit go.**

| Pri | Item | Notes |
|-----|------|-------|
| ✅ **P1 (adoption) — DONE + PUBLISHED** | **Language SDKs (TypeScript → Python → Go → Rust)** | REST + WebSocket clients under `sdk/`, independent SemVer from the server (publish only on an `sdk-*` tag). Frozen v1 method surface = `docs/Client Contract.md`; black-box scenarios (which also catch server bugs) = `docs/Client Testing.md`. **TypeScript ✅ (Cluster 294, 0.1.0)** — dependency-free `Client` (REST + WS), full `.d.ts`, `MaidanError`, `subscribe`/`waitFor*`, verified black-box (5/5) via `scripts/sdk-test.sh`. **Python ✅ (Cluster 295, 0.1.0)** — dependency-free (stdlib `urllib` REST + a hand-rolled RFC-6455 WS), snake_case surface, verified black-box (5/5). **Go ✅ (Cluster 296, 0.1.0)** — dependency-free (stdlib `net/http` REST + a hand-rolled RFC-6455 WS), service-struct surface, verified black-box (`go vet`/`gofmt` clean). **Rust ✅ (Cluster 297, 0.1.0)** — standalone crate (no `maidan-*` dep; small sync `ureq`/`tungstenite` stack, since std has no HTTP/TLS), service-handle surface, verified black-box (`clippy -D warnings`/`fmt` clean). **THE SDK ARC (294–297) IS COMPLETE** — TS, Python, Go, Rust at 0.1.0. Remaining follow-ups: **(a)** registry publishing — **machinery DONE (Cluster 298):** `sdk-release.yml` publishes on `sdk-{ts,py,rs,go}-vX.Y.Z` tags; `NPM_TOKEN`/`PYPI_TOKEN`/`CRATES_TOKEN` repo secrets loaded; all four dry-run-verified. **Remaining: push the `sdk-*-v0.1.0` tags to actually publish** (`docs/SDK Release.md`), then confirm the packages resolve. **(b)** SDK interop CI — **DONE (Cluster 299):** a report-only `sdk-interop` job boots a server and runs all four black-box suites via `scripts/sdk-test.sh`; **(c)** typed response models (all four currently return generic JSON) — still open (0.2). Rust client must NOT depend on `maidan-server`. MCP stays a URL (the LangChain/AutoGen door, Cluster 280), not a 4th library; A2A stays a recipe (`examples/a2a_interop.py`, Cluster 289). **✅ PUBLISHED (verified 2026-08-28):** all four are LIVE at 0.1.0 — PyPI `maidan` 0.1.0, npm `maidan` 0.1.0, crates.io `maidan` 0.1.0, `sdk-go-v0.1.0` module tag; all four `sdk-release` runs succeeded 2026-08-27 (secrets loaded). A second 0.1.0 upload is rejected. Remaining is only typed DTOs (0.2) + the `sdk/README.md` "0.0.1 name-hold" doc lie (→ 316 scrub). |
| **P2 (adoption)** | **Hosted playground** (`maidan.world/play`) | A try-it sandbox: ephemeral workspace + the two-agent hero loop (Cluster 278) in the browser. Detail in `docs/Adoption.md` §3 |
| **P3 (adoption)** | **Hosted cloud** (managed Maidan) | Later; multi-tenant hosting. `docs/Adoption.md` §4 |
| **P2** | **SDK interop CI** | A CI job running the `docs/Client Testing.md` scenario catalog across the SDKs once they exist (the report-only A2A interop job, Cluster 289, is the pattern) |

## Standing risks (still open)

- **Channel/thread authorization** — **CLOSED** (arc 159–165): enforced on read/write (REST+MCP), events (WS+MCP SSE), management (`channel:admin`), and references. Historical detail: for REST (**160**): `channel_members` (**159**) + `ensure_channel_access` gate every REST content route + search + workspace-context (private channels need a membership row; public + `__dm__` unchanged; creator auto-added). Surfaces: MCP **point-access** tools enforced (**161**); MCP **aggregate** reads filtered (**162**); WS/MCP subscribe grants verified against membership (**163**); `reference.rs` gated (**165**); the `channel:admin` membership-management API shipped (**164**); the **A2A JSON-RPC ingress** (`POST /a2a/v1/rpc`) now channel-gated on post + task-read (**179**). DM generic-route participant gap **CLOSED (180)** — `ensure_thread_access` → `ensure_dm_participant` (verified `maidan-auth/src/access.rs`); subscribe-grant self-assertion **CLOSED** (grants verified against `channel_is_member`, `subscribe_grants.rs`). Optional Postgres RLS defense-in-depth deferred (needs a per-connection GUC refactor on the shared `PgPool`; ADR in Decisions.md, Cluster 216). Legacy `/members/:id/mentions` + `/inbox` self-only: **assessed in 315 — the "session can read another's inbox" concern was a FALSE POSITIVE** (bearer-only routes, no `/ui/api` mount → sessions get 401; bearers are act-as-any by design). Defensive `ensure_acting_member` guards added anyway (no-op today; future-proofs a `/ui/api` mount).
- **At-most-once event bus (default path)** — transactional outbox (**10**), quarantine (**12**), HTTP outbox replay (**56**); NOTIFY duplicates/gaps possible on the optimistic path. **Mitigated:** opt-in `at_least_once` reconcile delivery (WebSocket **125**, MCP SSE **126**) is gap-free + at-least-once per `consumer_id`.
- **Bootstrap / `AUTH_DISABLED`** — high-impact misconfiguration. **Mitigated:** fail-closed (**157**) — `AUTH_DISABLED` needs the explicit `MAIDAN_ALLOW_INSECURE_NO_AUTH` ack and refuses boot otherwise (and always in production); compile-time strip (**91**) removes the path entirely in hardened (`--no-default-features`) builds.
- **Indexer staleness** — opt-in `INDEXER_STALE_SECS`.
- **PostgresBus listener** — best-effort recovery; `/health/ready` reflects errors.
- **SQLite semantic search** — brute-force cosine fallback; optional `sqlite-vec` feature for an index; HNSW is Postgres-only (by design, not a gap).
- **`hash-v1` default** — `openai-compatible` provider (v117) gives real semantics; `hash-v1` is the offline/dev default, not semantically meaningful. **→ Cluster 315 adds a boot `warn!`** so a stranger who leaves it unset isn't silently served near-random "semantic" results.
- **`rsa` advisory `RUSTSEC-2023-0071`** — ignored (RS256 id_token verify via openidconnect v4; no fixed `rsa`); clears on openidconnect v5 (unreleased). See [Dependencies.md](Dependencies.md).
- **No `v93`–`v100` tags** — clusters 93–101 shipped as one batch (PR #264), released as `v101.0.0`; not a backlog. All four gate tags (incl. `maidan-operator-1.0`) are cut.

## Shipped (reference)

| Ladder / tag | Highlights |
|--------------|------------|
| **17–27** | MCP fan-out, SQLite semantic, Helm server, purge, streamable subset |
| **35–58** | `maidan-2.0` product gate — DMs, webhooks, slash, FSM, erase, quotas, completion e2e |
| **59–67** | [[Agent Integration]], streamable TTL, A2A card, outbox ops, app OAuth, context |
| **68–76** | Automation DLQ, capability map, vault truth, A2A subscribe, MCP context, agent gate — [[Retros/Cluster 76.0]] |

**Release signing:** cosign keyless `sign-blob --bundle` over tarballs + SBOM in `release.yml` (automated; was previously manual).

## Still deferred (no separate owner)

| What | Notes |
|------|-------|
| Multi-region active-active | Out of scope |

_Closed (verified v126/v131/v132/v144/v148): OpenAPI↔capability map (**121**), OTLP export + dashboards + e2e (**89/90/123**), `sqlite-vec` + per-model embedding tables (**85/86**); webhook+automation delivery unification — substantially addressed (shared signing/backoff + unified operator API; storage intentionally separate, **131**); global cross-workspace admin audit query API (`GET /operator/audit`, gated by `audit:read-global`, **132**); docs link-checker in CI (`mdbook-linkcheck` gate, **144**); full MCP streamable transport spec-completeness (version negotiation + header + batching + notifications + GET SSE + `Accept` + resumability + server→client requests, arc **145–148**)._

## Known state

- **Latest tag:** **`v314.0.0`** (post-gate hardening, Phase XXIV). Since v273: MCP `2026-07-28` (300–303), durable mail retry (304–306), Slack + GitHub projectors (307–312), the SDK arc published at 0.1.0 (294–299), and launch-prep (313 default-secure quickstart, 314 claims/policies/release-verification). Next: the 2026-08-28 sweep's 315–318 + fidelity/context flagship arc (above). *(Narrative below is the historical v273 program record.)* Post-v155 four-arc program complete (156–178). **Security-led four-arc program: Arc A (security & correctness) COMPLETE** (179–184); **Arc B (multi-tenant SaaS ops) COMPLETE** (185–189); **Arc C (agentic task-queue depth) COMPLETE** (190–197). **Arc D — performance & scale**: tractable perf wins DONE — 198 load/soak harness (`scripts/loadgen.sh` + `#[ignore]`d `load_baseline`), 199 concurrent workspace-context assembly (bounded `buffered` per-thread builds), 200 filtered-ANN search (RBAC private-channel deny pushed into the query; honors `limit`, no leak), 201 workspace-sharded event fan-out (`ShardedBroadcast`; O(relevant) not O(all)). **Arc D remaining items — assessed + deferred, NOT abandoned:**
    - **Batched `pg_notify` — DECLINED (low value + delivery-core risk).** The LISTEN handler hydrates a single pointer per NOTIFY, and the hot path publishes per-event (no natural batch); only the latency-tolerant fallback relay batches. A correct coalescing needs **range-hydration** on the listener (track `last_hydrated_log_id`, hydrate `(last_hydrated, X]` per pointer, advance) — a delivery-core change for a win that only helps the non-hot path. Range-hydration alone is a robustness win (self-heals dropped NOTIFYs) if ever wanted, but risks double-delivery without careful last-hydrated tracking.
    - **Read-replica routing — DEFERRED (needs infra + a Store refactor).** Requires a second read-pool threaded through the `Store` (which is constructed with one pool), read-after-write consistency handling (route reads-after-writes / real-time to primary; only lag-tolerant reads like search/workspace-context to the replica), config (`MAIDAN_DATABASE_REPLICA_URL`, degrades to primary when unset), and a real replica to validate beyond the degenerate case.
- **Deferred from Arc C:** federation `content→parts` **egress** (194 did ingest; egress still body-only).
- **Perf follow-ups (surfaced this arc):** the workspace-context route builds every page thread then RBAC-filters (build-then-filter wastes work; filter-before-build is a bigger refactor with pagination subtlety); the search deny-set is `list_channels` + a per-channel `channel_is_member` (a single "my private channels" query would be cheaper); full DM-at-query-level for search (eliminating the post-filter) deferred (DM participation in SQL is complex).
- **NEW four-program arc (from a 5-agent sweep, 2026-08-12) — run in order, clusters 202+:** **(A) Security & correctness round 2** — 202 session-bound acting identity ✅, 203 DM/group-DM participation ✅, 204 cross-tenant artifact isolation (maidan_artifact_refs link table) ✅. **Transactional outbox** (atomic domain-write + event-append — the 184 deferral; `*_with_event` Store methods in one tx) is a multi-cluster migration: **205** foundation (`append_in_tx` + channel/thread create) ✅, **206** votes + reactions ✅, **207** pins + mentions ✅, **208** thread transitions (`transition_thread_with_event` + `thread_scope_in_tx`) ✅, **209** thread assignments (assign/unassign/claim/claim_next `*_with_event`; thread-scoped batch complete) ✅, **210** DM/group-DM posts (`post_message_with_event(new, dm_conversation_id)`) ✅, **211** the regular (slash-entangled) message post (`edit_message_with_posted_event` for the slash-finalize; no-slash uses `post_message_with_event`) ✅, **212** message **edit** + **tombstone** (`message.rs` now `publish()`-free) ✅, **213** the **A2A ingest** post (reuses `post_message_with_event`) + **member/workspace creation** ✅, **214** **references** (`add_reference_with_event`) + **artifacts** (`upsert_artifact_with_event` — folds upsert + the Cluster-204 access ref + `ArtifactUpserted` in one tx) ✅. **The domain-mutation outbox migration is COMPLETE (205–214)** — every event tied to a domain-table write commits atomically with it. `publish()` correctly **remains** (no rename/delete): its remaining callers append **standalone events** with no domain-table row to be atomic with — the federation **relay** (`federation.rs` re-publishes remote events onto the local bus) and **`publish_routed_mentions`** (`routes/mod.rs` fans a durable `MentionRecorded` to each auto-parsed @mention for realtime routing — no `maidan_mentions` row, distinct from the explicit-mention-API `record_mention_with_event` of 207). `publish()` = "durably append a standalone event + notify" is the right primitive for both, so the refactor concludes at 214 with no cleanup cluster. **215** **federation ingest trust policy** ✅ — `EventKind::federatable()` allowlist enforced at ingest (`ArtifactUpserted` excluded — blobs aren't federated; both push endpoint + pull worker) + fixed the `MemberJoined` remap leaking the peer's remote `member.workspace_id`. (The "referenced-entity-in-peer-workspace" framing resolved to that nested-workspace re-scope fix; federation is event-log replication, not entity materialization, so there are no local entity rows to validate against.) **216** the **RLS spike** ✅ — resolved as a decision ADR (`docs/Decisions.md` `## Security`): Postgres Row-Level Security assessed + **deferred**; app-layer RBAC stays authoritative (blockers: shared pool with no per-request tenant binding, workspace-agnostic `Store` trait, SQLite has no RLS → parity break, cross-workspace bearer orchestrator model, duplicates an already-comprehensive control; trigger conditions recorded). **Program A (security & correctness round 2, Clusters 202–216) is COMPLETE.** **(B) agentic orchestration** (task DAG, scheduled/recurring tasks, capability registry + skill routing, queue depth, coordination waits + structured results) — **BEGUN: 217** landed the **task-dependency DAG store foundation** ✅ (`maidan_thread_dependencies` edges + `thread_deps` store: add/remove/list-deps/list-dependents/`dependencies_satisfied`; readiness = all deps terminal; reuses the thread-as-task model; zero-blast-radius, no routes yet — the Cluster-159 pattern). **218** readiness-aware `claim_next` ✅ (a `NOT EXISTS` clause skips tasks with non-terminal deps, both backends + `_with_event`; the existing REST claim-next route + MCP `claim_next_thread` tool are now DAG-aware, no new API). **219** DAG-management REST API ✅ (`POST/GET /threads/:id/dependencies` add + list+`ready`, `DELETE …/:dep_id`, `GET /threads/:id/dependents`; both-thread RBAC + same-workspace; full new-route preflight). **220** the MCP DAG tools ✅ (`add_thread_dependency`, `list_thread_dependencies`; both-thread RBAC; DAG read/write surface complete over REST + MCP). **221** transitive cycle prevention ✅ (`add_thread_dependency` rejects direct + transitive cycles via recursive-CTE reachability, both backends — the DAG is now acyclic). **222** reactive readiness ✅ (`ThreadReady` event on dependency-unblock + `newly_ready_dependents` query, both backends; subscribe with `kinds=thread_ready`). **223** `wait_for_ready` MCP long-poll ✅ (blocks until a task becomes claimable; the `wait_for_mention` analogue; DAG surface now complete end-to-end). **224** channel queue-depth ✅ (`GET /channels/:cid/queue-depth` → ready/assigned/blocked counts; one aggregate query, both backends). **225** `get_queue_depth` MCP tool ✅ (the MCP twin; task-queue subsystem now feature-complete over REST + MCP). **226** scheduled/recurring task foundation ✅ (`task_schedules` table + model + store CRUD/due-scan, both backends; zero-blast-radius, no worker/routes). **227** scheduler sweeper worker ✅ (opt-in `MAIDAN_SCHEDULER_TICK_SECS`; `claim_next_due_schedule` atomic claim-and-advance, `FOR UPDATE SKIP LOCKED` on pg so replicas don't double-fire; fires a task thread per due schedule; at-most-once on crash). **228** scheduler REST management ✅ (create/list/pause-resume/delete over `/workspaces/:wid/task-schedules` + `/task-schedules/:id`; `workspace:write` + target-channel access; `set_task_schedule_active`). **229** scheduler MCP ✅ (`create_task_schedule` + `list_task_schedules`; the scheduled/recurring-task subsystem is now complete over REST + MCP). **Arc E — capability registry + skill routing** opened: **230** member-skills foundation ✅ (`member_skills` table + model + store add/remove/list, both backends; zero-blast-radius). **231** skill-aware claim ✅ (`thread_required_skills` + `claim_next`/`claim_next_with_event` skill-match clause, both backends; existing claim route/tool inherit skill routing). **232** capability-registry REST ✅ (member-skill + thread-required-skill CRUD; 6 routes). **233** capability-registry MCP ✅ (add/list member skills + add/list thread required-skills). **Arc E COMPLETE** (230 foundation → 231 skill-aware claim → 232 REST → 233 MCP). **Deferred:** a "capable members for this task" discovery read (members whose skills ⊇ requirements) — optional orchestrator convenience; `claim_next` already routes automatically. **Arc F — coordination waits + structured results** opened: **234** structured-results foundation ✅ (`thread_results` table + model + store set/get, both backends; zero-blast-radius). **235** Arc F REST + event ✅ (`PUT /threads/:id/result` `thread:transition` upsert + `GET …/result` `workspace:read` → `404` until produced, both DM-participant-aware thread RBAC; `ThreadResultSet` event on set — a "go fetch" pointer observable on WS + MCP-SSE like `ThreadReady`, locally-derived → non-federatable). **236** Arc F MCP ✅ (`set_thread_result` `thread:transition` / `get_thread_result` `workspace:read` — the twins of 235's REST; **`wait_for_result`** `workspace:read` — block on a thread's `ThreadResultSet`, return the result payload, the `wait_for_ready` analogue; **`get_dependency_results`** `workspace:read` — a parent aggregates its dependencies' outputs as `[{thread_id, result}]`, RBAC-filtered; 5-place MCP wiring + both sorted contracts; test `result_tools_set_get_wait_and_aggregate`). **ARC F COMPLETE (234–236) — and PROGRAM B (agentic orchestration, 217–236) is COMPLETE**: task-DAG + queue (217–225), scheduled/recurring tasks (226–229), capability registry + skill routing (Arc E, 230–233), coordination waits + structured results (Arc F, 234–236). **Deferred within Program B:** a "capable members for this task" discovery read (Arc E note); federation egress `content→parts` (A2A ingress `parts→content` shipped 194). Next: **Program C (notifications & reach)**, then **Program D (scale & durability)**. **(C) notifications & reach** (per-recipient router + inbox, prefs + presence-aware routing, email/SMTP transport, digests, follow/UI) — **BEGUN** (grounded by a fresh recon of the mentions/webhook/presence/subscribe surface): the gap is that mentions are *recorded + polled*, never *delivered per-recipient*; webhook delivery is a single per-workspace firehose keyed on event kind; no prefs/mute/follow; `deliver_http` is the only transport. Planned as three arcs (plan in scratchpad `program-c-plan.md`): **Arc G** per-recipient ledger + router + unified inbox, **Arc H** preferences + subscription (mute/follow), **Arc I** transport (email/SMTP) + digests + presence-aware routing + `/ui` notification center. **237** ✅ opened Arc G with the per-recipient notification ledger foundation (`maidan_notifications` pg 0042/sqlite 0041 — one row per recipient × source event, `kind`=`EventKind`, `source_log_id` no-FK so it survives retention pruning, denormalized `channel/thread/message/actor`, `read_at` NULL=unread; `Notification`/`NewNotification` + store CRUD both backends; zero-blast-radius, no router/routes). **238** ✅ notification router (`NotificationRouter` always-on reconnecting bus consumer in `main.rs`; `route_event` resolves `MentionRecorded`→mentioned member, channel resolved from thread; `create_notification_if_absent` + `UNIQUE(member_id,source_log_id)` index pg 0043/sqlite 0042 → cross-replica/replay dedup; `maidan_notifications_created_total{kind}` metric; e2e `notification_router_e2e`). **239** ✅ REST unified inbox (`GET /members/:id/notifications` list + `…/unread-count` + `POST …/:nid/read` + `…/read-all`; all `workspace:read` + **self-only** for sessions via `ensure_acting_member`, bearer act-as-any; `mark_notification_read` recipient-scoped `(member_id,id)` in the store; full new-route preflight; e2e `notifications_inbox_e2e`). **Follow-up surfaced:** the legacy `/members/:id/mentions` + `/inbox` routes enforce only `workspace:read` + same-workspace (any workspace member can read another's mention feed) — the Cluster-202/203 self-only hardening never reached them; retrofit them (not done in 239 to keep scope tight). **240** ✅ MCP `list_notifications`/`get_unread_count`/`mark_notification_read` (twins of 239's REST) + **`wait_for_notification`** (general form of `wait_for_mention`; shared `wait_for_member_event` helper; returns the triggering event, ledger backs the drain). **ARC G COMPLETE (237–240): per-recipient notification ledger → router → REST inbox → MCP.** **Remaining Program C: Arc H** preferences + subscription — **241** ✅ mute-preferences foundation (`maidan_notification_prefs` pg 0044/sqlite 0043, PK `(member_id,kind)` + `muted`; `NotificationPref` + store set/list/`is_notification_muted`; zero-blast-radius). **242** ✅ mute-aware router + prefs REST (`route_event` consults `is_notification_muted` → skip muted `(member,kind)` + `maidan_notifications_suppressed_total{reason}` metric; `PUT`/`GET /members/:id/notification-prefs` set/list, `workspace:read` + self-only). **243** ✅ mute MCP tools (`set_notification_pref`/`list_notification_prefs`; `kind` snake_case parsed → EventKind; member-scoped, no gate arm) — **the mute half of Arc H is complete over REST + MCP.** Remaining Arc H = **follows/subscription**: **244** ✅ foundation (`maidan_channel_follows`+`maidan_thread_follows` pg 0045/sqlite 0044, presence=following, reverse index; `ChannelFollow`/`ThreadFollow` + store follow/unfollow/list/`*_followers`, both backends; zero-blast-radius). **245** ✅ follows-aware router + REST (`route_event` `MessagePosted` arm fans to `channel_followers ∪ thread_followers` minus author, mute-checked via a shared `notify` helper; skips DM posts; `POST`/`GET /members/:id/channel-follows` + `DELETE …/:cid` + thread triple, self-only, follow gated on `ensure_channel/thread_access`). **CORRECTION to the earlier note:** the dedup index does NOT prevent a mentioned-and-following member getting two notifications — `MentionRecorded` and `MessagePosted` are distinct events (distinct log_ids); per-kind mute (`message_posted`) is the control. Follow-up: the router doesn't skip followers who LOST access after following (pointer-only notification; thread read stays RBAC-gated). **246** ✅ follows MCP tools (`follow_channel`/`unfollow_channel`/`list_channel_follows` + thread triple; `follow_*` gate on target access via the pre-dispatch channel/thread arms). **ARC H COMPLETE (241–246): mute preferences + follows/subscription over REST + MCP.** Remaining Program C: **Arc I** (transport + reach) — **247** ✅ email/SMTP transport foundation (`MailTransport` trait + `lettre` `SmtpTransport` + `SmtpConfig::from_env`; config-gated + unwired; `lettre` on the rustls+tokio stack, `cargo deny` green with `0BSD` allowed). **248** ✅ recipient-address store (`maidan_member_emails` pg 0046/sqlite 0045, one per member — separate table to avoid the member-row ripple; `MemberEmail` + set/get/delete). **⚠️ 248 also carried an mdbook hotfix:** the Cluster-236 `get_dependency_results` catalog description used bare `[{thread_id, result}]`, which `gen-mcp-reference` renders into `mcp-reference.md` prose where the mdbook linkcheck treats it as an incomplete link (memory `maidan-docs-linkcheck-brackets`) → the non-required `mdbook` job had been RED since 236 (unnoticed because the ship-monitors only watch the 8 required checks). Fixed the description to prose (no brackets). **Lesson: also glance at `mdbook` (+ other non-required jobs) before merging, not just the 8 required.** **249** ✅ email delivery wired into the router (`AppState.mail` + `attach_mail`, built from `SmtpConfig::from_env` in `main.rs`; router `deliver_notification_email` spawned best-effort after the in-app write so a slow SMTP server never blocks routing; `maidan_email_delivered_total{outcome}`; address-presence = opt-in; recording-transport e2e). **Best-effort, no retry** (a durable retrying email delivery queue is a follow-up); **no address surface yet** (set via store only until 250). **250** ✅ member delivery-email REST (`PUT`/`GET`/`DELETE /members/:id/email`, self-only + light `@` check; email now usable end-to-end over REST). **251** ✅ `/ui` notification center (a "Notifications" tab: list + unread badge + mark-read/read-all + unread-only filter, over `/ui/api/members/:id/notifications*` routes reusing the 239 handlers under session middleware; `sessionMemberId`=self; `ui_js_contract` green). **252** ✅ durable member last-seen store foundation (`maidan_member_last_seen` pg 0047/sqlite 0046, `member_id` PK + `last_seen_at`; store `touch` upsert-`now()` / `get` → `Option<DateTime>`, both backends — the persistent presence signal presence-aware routing needs since presence is in-memory only; separate table to avoid the member-row ripple, no model type; zero-blast-radius, unwired until 253). **253** ✅ presence-aware email routing — the WS handler `touch`es `last_seen` on presence registration (at the `ws.rs` `register` call site, NOT inside the store-less `PresenceHub`; best-effort + spawned so it never blocks the connect), and `deliver_notification_email` skips the send when the recipient was seen within `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS` (opt-in; unset/0 = send as before, Cluster-249 behaviour), metered `outcome="skipped_present"`, fail-open on a read error. Wires the 252 store end-to-end. **254** ✅ scheduled-digest **data model** (store foundation): user chose the alternative-mode product (immediate per-notification emails OR a periodic digest, not both), so this landed `EmailDeliveryMode` (`Immediate` default / `Digest`) + `DigestDue` in maidan-types, `maidan_member_delivery_prefs` + `maidan_member_digest_state` (pg 0048/sqlite 0047), and store `set/get_delivery_mode` (default Immediate on absence) / `set_last_digest_at` (digest watermark) / `members_due_for_digest` (digest-mode members w/ address + unread-since-last-digest, address inline), both backends — zero-blast-radius, unwired. **255** ✅ wired it — the router skips a digest-mode member's immediate email (`deliver_notification_email` early-returns on `get_delivery_mode == Digest`, metered `skipped_digest`), and an opt-in digest sweeper worker (`digest.rs`, `MAIDAN_DIGEST_TICK_SECS`, Cluster-227 sweeper shape) drains `members_due_for_digest`, emails an unread-count rollup via `state.mail`, and advances `set_last_digest_at` **only on a successful send** (at-least-once, self-healing — a transient failure retries next tick). No-op without a transport; **deliberately NOT single-flighted** across replicas (a duplicate digest is low-harm, unlike the scheduler's harmful double-fired task — run on one replica for exactly-once). Alternative-mode digest works end-to-end. **256** ✅ delivery-mode **REST** — `PUT`/`GET /members/:id/delivery-mode` (`workspace:read` + self-only via `ensure_acting_member`, the notification-prefs cap model); `SetDeliveryMode` wraps `EmailDeliveryMode` so an unknown mode is a `400` at the extractor; `GET` is total (`immediate` default, no 404); full new-route preflight (OpenAPI + `EmailDeliveryMode`/`SetDeliveryMode`/`DeliveryModeView` schema regs + capability-map + matrix PUT body clause). **257** ✅ delivery-mode **MCP** tools (`set_delivery_mode` / `get_delivery_mode`, `workspace:read`, member-scoped, no gate arm — the notification-pref tool shape; `set` parses snake_case `immediate`/`digest` → `InvalidParams` on unknown, both return `{mode}`) — the twins of the 256 REST. **The core of Arc I is complete** (transport 247 → address store 248 → router wiring 249 → address REST 250 → `/ui` center 251 → presence-aware routing 252–253 → digests 254–257). **Remaining Arc I:** **optional** MCP email-address tools for parity (low value — email is human-facing config). **Arc I** (email/SMTP transport + digests + presence-aware routing + `/ui` notification center). **(D) scale & durability** — **BEGUN (user chose the NOTIFY floor first, 2026-08-21): 258** ✅ event-bus self-healing NOTIFY floor (`maidan-bus/postgres.rs`: high-water `log_id` + `drain_new_events` back-fills the missed range from the log on a gap (pointer id > `high_water+1` → back-fill the exclusive middle) or a listener reconnect (drain to head); always single-hydrates the pointer's own id so a concurrent late-lower id isn't dropped; batched/best-effort; `list_after_global`/`max_event_id` cross-workspace log reads; `Backfilled` stat + `{result="backfilled"}` metric; `PostgresBus::backfill` heal hook. Optimistic-path resilience — the outbox + at-least-once cursor stay the durable path). **259** ✅ chaos / fault-injection harness (`crates/maidan-bus/tests/chaos.rs` + `scripts/chaos.sh`): an `#[ignore]`d soak publishes under load while killing the `LISTEN` backend (`pg_terminate_backend` on `LISTEN` connections), asserting published ⊆ delivered — validated the 258 floor end-to-end (40/40 delivered across 5 kills, 0 missing). Pure `fault_due` cadence helper unit-tested in CI; soak is a manual tool like `loadgen`. **260** ✅ backup/restore + DR runbook (`scripts/backup.sh` = `pg_dump -Fc` + tar of the localfs artifact root + manifest; `scripts/restore.sh` = `pg_restore`, refuses a non-empty target without `--force`; a "Backup & disaster recovery" section in `docs/Production.md` covering out-of-band secrets, S3-is-durable, RPO/RTO, recovery steps — operator tools like `loadgen`/`chaos`, `bash -n`-clean, not CI-gated). **Read-replica routing — IN PROGRESS** (user chose the full **LSN causality-token** design, 2026-08-22: strong read-your-writes, multi-cluster, validated against real streaming replication; plan in scratchpad `read-replica-plan.md`). **261** ✅ LSN primitives + replication harness (validate-first keystone): `Lsn` token type (`maidan-types`, u64-backed for correct numeric ordering) + store `current_wal_lsn`/`replica_replay_lsn`/`replica_caught_up` (`postgres::replication`, direct-call like `get_by_id`) + `scripts/replica-harness.sh` (proven local pgvector primary+standby recipe — pg_hba `host replication` line + standby `pg_basebackup -R` as the postgres user) + an `#[ignore]`d test validating the helpers against real replication (passed). Inert — no read routed yet. **262** ✅ reader-pool split (`PostgresStore { pool, reader }` + `with_replica_reader`; `new` defaults reader=primary so no ripple to ~62 `new` sites; `MAIDAN_DB_REPLICA_URL` config + boot wiring connects a real reader pool, fail-fast on a bad URL, shared connection setup; reads still on the primary — inert until 264; `reader` field `#[allow(dead_code)]` until the selector). **263** ✅ consistency token on writes: `Store::write_lsn()` (Postgres `pg_current_wal_lsn()`, SQLite `None`) + `AppState.read_replica_enabled` (main.rs from `MAIDAN_DB_REPLICA_URL`) + `consistency::middleware` stamping `Maidan-Consistency-Token: <lsn>` on successful mutations, captured after the handler (safely over-approximating — never behind the write), gated on a configured replica (no replica → no token, no round-trip). **264** ✅ token ingestion + read routing: `READ_CONSISTENCY` task-local + `with_read_consistency` (GET/HEAD-only scope, so mutation/background reads stay on the primary — no read-then-write staleness) + `read_pool()`/pure `route_decision` + a background poller caching the replica's `pg_last_wal_replay_lsn()` in an atomic (stale cache is safe — only false-routes to primary) + entity-read delegations (workspace/member/channel/thread/message get+list) routed to the replica once it has replayed past the client's token, else primary. **Validated vs real streaming replication** (`read_routing` ignored e2e passed: read-your-write holds, replica serves no-token reads). **265** ✅ routed the remaining content/collaboration read families (28 delegations: skills/results/notifications/follows/emails/last-seen/channel-members/dm/group-dm/transitions/queue-depth/schedules/assigned/deps/edits/mentions/inbox/votes/reactions/usage) + `maidan_replica_reads_total{outcome}` (store-side `ReadRoutingMetrics`). **Auth-path reads (sessions/tokens/oidc/peers) + control-plane/config reads (webhooks/slash/fsm-hooks/deliveries/reindex/audit/quotas) deliberately stay on the primary** (auth middleware runs on GETs → a lagging replica would break just-minted creds). Validated vs real replication (routing counters assert both outcomes). **266** ✅ replica-lag gauge (`maidan_replica_lag_bytes` — poller samples primary write LSN too → `current − replay`) + Production.md "Read replicas" section (config, `Maidan-Consistency-Token` contract, routing policy, metrics, harness). **The LSN read-replica arc (261–266) and PROGRAM D (scale & durability) are COMPLETE** — and with them the entire security-led four-program run (A 202–216, B 217–236, C 237–257, D 258–266). **Optional-deferrals sweep IN PROGRESS** (user chose: import BOTH modes, search HONOR-the-token; 2026-08-24 — scratchpad `deferrals-plan.md`). **267** ✅ A2A egress `content→parts` (`message_parts_from_content` egress inverse + the A2A agent renders its outbound message from the stored message's canonical content, not an echo; federation event-relay already carried content). **268** ✅ MCP email-address tools (`set/get/delete_member_email`, parity w/ 250 REST over the 248 store). **269–270** ✅ workspace import (both modes: new-workspace-remap default / `?mode=restore` same-id, `&force` erases first) — `Store::import_workspace` + `POST /workspaces/import`. **271–272** ✅ search token-aware read routing (`PostgresSearch` reader pool + replay poller + shared `maidan_store::postgres::replica_route`; `maidan_search_replica_reads_total` metric) — validated vs real streaming replication. **The optional-deferrals sweep (267–272) and the LSN read-replica program are COMPLETE.** Transactional outbox already DONE (shared w/ Program A, 205–214). Full per-lens detail in the session's workflow journal `wf_b8cdaaa2-be4`. **Next forward work → see "Post-272 forward work" below.**
- **Assignment queue follow-ups (Clusters 190–192):** MCP tools shipped in 191; claim leases + reclaim shipped in 192. Remaining: `claim_next` is channel-scoped (no workspace-wide pull); no server-side default lease (the caller sets `lease_secs`); reclaim is lazy (only a subsequent `claim_next` frees an expired lease — nothing actively unassigns a dead holder / emits an event until someone pulls).
- **Secret-rotation follow-ups (Cluster 189):** migration to a rotated key is lazy (a secret moves only when re-saved — no bulk re-encrypt sweep, so an old key must stay in `FEDERATION_DECRYPT_KEYS` until all secrets rotate); the fallback set is a startup `OnceLock` (rotation needs a restart, not a live reload).
- **Usage/metering follow-ups (Cluster 188):** no per-tenant storage bytes (content-addressed artifacts dedup across workspaces — attributing by uploader would double-count; decide a convention if billing needs it); usage is a point-in-time snapshot (no historical time-series — operators sample on their cadence).
- **Workspace export follow-ups (Cluster 187):** reactions/votes not exported (per-message N+1); artifact blobs not included (metadata via references only); the bundle is built in memory + returned in one response (a streaming/NDJSON variant would scale better). *(Import path SHIPPED 269–270 — `Store::import_workspace` + `POST /workspaces/import`, both new-workspace-remap and `?mode=restore`; the "no import path yet" note is resolved.)*
- **Retention follow-ups (Cluster 186):** no `occurred_at` index on the pruned tables (the daily batched sweep tolerates a scan; add if it gets hot); deliveries prune is lightly tested (valid-query/empty smoke — the FK fixture for delivery rows was deferred); a stale/abandoned delivery cursor pins the event-log prune floor (needs a stale-cursor reaper eventually).
- **Denial (401/403) auditing → logs/metrics, not the audit table (Cluster 182 decision).** Table-level per-denial auditing is an attacker-controlled, unbounded `maidan_audit` write amplifier. If durable denial history is ever needed, do it in a sampled/rate-limited sink separate from the audit table.
- **True single-transaction dual-write atomicity — ✅ DONE (transactional-outbox migration 205–214).** *(Corrected 2026-08-28: this was listed as open, but the migration completed it.)* Every event tied to a domain-table write now commits atomically with it via `*_with_event` store methods sharing one tx (verified e.g. `postgres/channels.rs` `create_with_event` = `begin → append_in_tx → commit`), including the slash-entangled message-post path (`edit_message_with_posted_event`). `publish()` correctly remains only for the two callers that append **standalone** events with no domain row to be atomic with (the federation relay + `publish_routed_mentions`).
- **Deferred (Cluster 173):** federation/A2A-ingested messages carry `body` only — the ingest path (`a2a_agent.rs`, federation worker) doesn't yet map incoming `parts → content` (typed structured content). In-scope-to-not-break; propagation is a follow-up. All four gate tags cut (`maidan-2.0` v58, `maidan-agent-1.0` v76, `maidan-operator-1.0` v101, `maidan-scale-1.0` v120).
- **Active work:** post-gate hardening clusters (121+); no further ladder gate defined. See [[Roadmap]] + [[Remaining Work]].
- **Integrators:** start at [[Agent Integration]] and `contracts/`.

## How to read this file

- **[[Remaining Work]]** — partial implementations + Slack matrix.
- **[[Roadmap]]** — cluster pointer and historical closes.
- Retro PRs are the right time to add or remove deferrals.
