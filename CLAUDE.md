# Agent guide

If you are an AI agent or human dev landing in this repo for the first
time, read this file end-to-end before doing anything else. It is the
single source of truth for *how* to operate in this codebase. The
*what* lives in [`docs/`](docs/) — integrators use
[`docs/Integration.md`](docs/Integration.md); contributors use
[`docs/README.md`](docs/README.md) after this page.

## 30-second orientation

- **Name:** Maidan. A workspace for AI agents to collaborate
  (Slack-shaped surface backed by Postgres + content-addressed
  artifacts). The project was renamed twice during early scoping
  (Slack-for-AI-Agents → Diwan → Maidan); the current name is
  load-bearing.
- **Language:** Rust 2021, toolchain pinned via `rust-toolchain.toml`
  (currently 1.91). Workspace with 13 member crates.
- **Owner:** `david-engelmann`. Solo maintainer. Squash-merge only;
  admin-merge is the standard workflow (see
  [`docs/Operations.md`](docs/Operations.md)).
- **Release cadence:** work ships in clusters — the initial A–H + 1.0
  arc (`v0.X.Y` → `v1.0.0`), then a numbered product ladder (1–120,
  tagged `vX.0.0`). Every cluster closes with a mandatory retro PR and
  a tag. Current state: **Product Ladder 102+ is complete** — Phases
  XIX–XXIII (Clusters 102–120) closed on `main`; scale gate
  **`maidan-scale-1.0`** at **`v120.0.0`**. No further *ladder* cluster
  is defined past 120; subsequent clusters are **post-gate hardening**
  (Phase XXIV, **Cluster 121+**, latest **`v167.0.0`**, tagged `vX.0.0` on
  the same ladder but with no new gate tag — see "Project state at this
  handoff" below and [`docs/Roadmap.md`](docs/Roadmap.md)).
- **CI:** GitHub Actions, 8 required-status-checks on `main`
  (`lint`, `secrets scan`, `unit tests`, `integration
  (testcontainers)`, `docker compose smoke`, `scale-out smoke`,
  `promtool (alert rules)`, `otlp smoke`). Every PR runs all 8.
  (`scale-out smoke` was promoted at the `maidan-scale-1.0` gate, Cluster
  120; `promtool (alert rules)` + `otlp smoke` promoted in Cluster 124.)

## Read order

**External integrators (not editing this repo):** [`AGENTS.md`](AGENTS.md) →
[`docs/Integration.md`](docs/Integration.md) → published
[mdBook](https://david-engelmann.github.io/maidan/) — skip `docs/Clusters/`.

**Repo contributors:**

1. **This file** — operating manual.
2. [`docs/README.md`](docs/README.md) — doc index.
3. [`docs/Architecture.md`](docs/Architecture.md) — components and data flow.
4. [`docs/Capabilities.md`](docs/Capabilities.md) — what ships in which release.
5. [`docs/Decisions.md`](docs/Decisions.md) — load-bearing ADRs.
6. [`docs/Operations.md`](docs/Operations.md) — PR flow, CI, releases.
7. [`docs/Open Work.md`](docs/Open%20Work.md) — backlog and risks.
8. [`docs/Roadmap.md`](docs/Roadmap.md) / [`docs/Retros/`](docs/Retros/) — when doing cluster work.

## The cluster model in one paragraph

Work is sliced into **clusters** (A through H plus 1.0). Each cluster
delivers a coherent capability (`v0.0.1` foundation, `v0.1.0` routing
+ bus + MCP, `v0.2.0` search, etc.). Within a cluster, work is a
small numbered sequence of PRs (C.1, C.2, …). Every cluster closes
with a `[X.retro]` PR that writes `docs/Retros/Cluster X.md`,
prepends a new section to [`docs/Capabilities.md`](docs/Capabilities.md),
adds a `[v0.X.0]` section to [`CHANGELOG.md`](CHANGELOG.md), refreshes
[`docs/Architecture.md`](docs/Architecture.md) and the "Current
cluster" pointer in [`docs/Roadmap.md`](docs/Roadmap.md), then the
maintainer tags `v0.X.0` and pushes — which triggers
[`.github/workflows/release.yml`](.github/workflows/release.yml). The
retro is mandatory. The tag does not get cut without it.

## PR workflow (the short version)

1. Open a GitHub Issue from the relevant template *or* link an
   existing cluster-phase issue (each cluster's plan in
   `docs/Clusters/Cluster X.md` lists the issues).
2. Branch from `main`: `<kind>/<scope>-<slug>` per
   [`docs/Conventions.md`](docs/Conventions.md). Examples:
   `feat/maidan-search`, `ci/release-darwin-x86`, `docs/cluster-c-retro`.
3. Develop on the branch. Locally run `cargo fmt --check`,
   `cargo clippy --all-targets --workspace -- -D warnings`, and
   the relevant test target (`cargo test -p <crate>`).
4. Commit with a Conventional Commits title (`feat(scope):`,
   `chore:`, `ci:`, `docs(retro):`).
5. `git push -u origin <branch>` and open the PR with `gh pr create`.
   The body **must** include the PR-level retro section per
   [`docs/Conventions.md`](docs/Conventions.md).
6. Wait for the 8 required CI jobs to pass. Use `gh pr checks <num>`
   or arm a Monitor.
7. Merge with `gh pr merge <num> -R david-engelmann/maidan --squash
   --admin --delete-branch`. The `--admin` flag is intentional and
   authorized — see [`docs/Decisions.md`](docs/Decisions.md) entry
   "Admin-merge instead of local-first push".
8. Sync local main: `git checkout main && git pull --ff-only && git
   branch -d <branch>`.

The full version is in [`docs/Operations.md`](docs/Operations.md).

## Test conventions you must know

- **Postgres testcontainers run against `pgvector/pgvector:pg17`**,
  not stock `postgres:11` (the default). Migration 0003 needs the
  `vector` extension. Pattern:

  ```rust
  use testcontainers::{runners::AsyncRunner, ImageExt};
  use testcontainers_modules::postgres::Postgres;

  let container = match Postgres::default()
      .with_name("pgvector/pgvector")
      .with_tag("pg17")
      .start()
      .await
  {
      Ok(c) => c,
      Err(err) => {
          eprintln!("skipping: docker unavailable ({err})");
          return;
      }
  };
  ```

- **Postgres tests skip gracefully if Docker is unavailable** —
  every integration test that uses testcontainers wraps `.start()`
  in a `match` with `eprintln!` + `return` on Err. Do not panic; CI
  for fork PRs may run without Docker.
- **SQLite tests use `sqlite::memory:`** with `PRAGMA foreign_keys =
  ON` explicitly turned on (off by default in SQLite).
- **Shared assertions go in `tests/common/mod.rs`**. Each test crate
  in the workspace that needs the pattern has its own copy
  (`maidan-store/tests/common/mod.rs`, `maidan-search/tests/common/mod.rs`).
  Both backends in each crate exercise the same suite from `common`.
- **Test names are descriptive sentences**, not action_under_test
  (`semantic_search_orders_by_cosine_distance`, not
  `test_semantic`).
- **No `tokio::sync::Notify::notify_waiters()` for cross-task
  signaling between a producer and a poller.** It only wakes
  *current* waiters. Use a polling loop instead — see
  `LoggingHandler::wait_for` in
  [`crates/maidan-search/src/indexer.rs`](crates/maidan-search/src/indexer.rs).

## Editing gotchas you must know

- **`Edit` requires `Read` first** for any file you intend to edit.
  This is enforced; a second `Edit` against a freshly-written file
  may fail if a linter (`cargo fmt`) touched it in between — re-Read
  the relevant range.
- **`cargo fmt` rewrites files**. It will reorder imports
  alphabetically and shift line breaks. After `cargo fmt && cargo
  fmt --check`, expect a notification that tracked files were
  modified by the linter — don't revert.
- **`Bash sed -i ''` for in-place edits on macOS** needs a backup
  extension argument: `sed -i.bak '...' file && rm file.bak`. Always
  clean up `.bak` after the substitution.
- **`Bash` auto-backgrounds long commands**. `cargo test` for full
  workspace can take several minutes; use the `run_in_background`
  parameter and the task notification, or `Monitor` for streamed
  results. Don't sleep-and-poll.

## Conventions that are not optional

- **No comments that restate code.** Only comment *why*, not *what*.
- **No `unwrap()` in library code** (`crates/maidan-*/src/`). Tests
  may unwrap freely.
- **`thiserror` for library errors**, `anyhow` only at binary
  boundaries.
- **`tracing` for logging** — no `println!` in library code.
- **Path deps inside the workspace** are fine and pinned via
  `publish = false` on every member crate (workspace-level
  `publish.workspace = true` inheritance). Don't change this without
  reading the `cargo-deny` decision in
  [`docs/Decisions.md`](docs/Decisions.md).
- **Squash-merge only**. Every PR's body becomes the squash commit's
  body — the PR-level retro lives there too.

## What you must not do

- **Do not commit secrets.** `.env`, `*.pem`, `*.key`, `maidan.toml`
  are git-ignored. CI runs `trufflehog`.
- **Do not bypass GPG signing** unless explicitly authorized. No
  signing key is configured (tags through `v167.0.0` are annotated but
  unsigned); annotated unsigned tags are acceptable until a key is set
  up.
- **Do not push to `main` directly.** Branch protection blocks it;
  even admins must PR.
- **Do not skip required CI checks** without explicit user
  authorization. Admin-merge with red CI is bypassing required-
  status-checks; only do it when the user has acknowledged the
  reason and authorized.
- **Do not introduce backwards-compatibility shims pre-1.0.** We
  rename, delete, and refactor freely until `v1.0.0` ships.

## When you are stuck

- The most recent `docs/Retros/Cluster X.md` is the freshest record
  of the project's shape and tension points. Read it.
- The `docs/Clusters/Cluster X.md` files document each cluster's PR
  ladder, ordering rationale, and risks.
- Every Cargo crate has a doc-comment at the top of `src/lib.rs`
  that explains its role and what's deferred.
- For decisions whose rationale isn't obvious, check
  [`docs/Decisions.md`](docs/Decisions.md).

## Project state at this handoff

- **Integrator docs:** [`docs/Integration.md`](docs/Integration.md) + [mdBook](https://david-engelmann.github.io/maidan/) (GitHub Pages).
- **Product Ladder 102+ is COMPLETE:** Phases XIX–XXIII (Clusters 102–120) merged on `main`. Scale gate **`maidan-scale-1.0`** tagged at **`v120.0.0`** (see [`docs/Gates/maidan-scale-1.0.md`](docs/Gates/maidan-scale-1.0.md)). No further ladder cluster is defined past 120; remaining work is post-gate human-product + cross-cutting tracks ([`docs/Open Work.md`](docs/Open%20Work.md), [`docs/Remaining Work.md`](docs/Remaining%20Work.md)).
- **Post-gate hardening (Phase XXIV, Cluster 121+):** opportunistic backlog burn-down tagged on the same `vX.0.0` ladder, no new gate tag. **Cluster 121** (`v121.0.0`) closed the OpenAPI-wide capability map in CI (Cluster 69 deferral) and extended the SLO dashboards/alerts to the Cluster 116 indexer metrics. **Cluster 122** (`v122.0.0`) added a `promtool (alert rules)` CI job that executes the SLO PromQL (it caught a `$value`-rendering bug in `MaidanIndexerQueueSaturated`) and corrected the OTLP-export status (shipped in Cluster 89, not open). **Cluster 123** (`v123.0.0`) added an `otlp smoke` CI job + `otlp` compose profile that proves OTLP traces + metrics reach a real collector end-to-end. **Cluster 124** (`v124.0.0`) consolidated the SLO-rule validators (one script) and promoted `promtool (alert rules)` + `otlp smoke` to required checks (**8 required** now). **Cluster 125** (`v125.0.0`) added opt-in at-least-once event delivery (`at_least_once` subscribe flag → cursor-driven reconcile over a stability horizon; default optimistic path unchanged). **Cluster 126** (`v126.0.0`) extended `at_least_once` to the MCP SSE transport (`/mcp/stream`). **Cluster 127** (`v127.0.0`) reconciled the backlog docs against code (struck ~11 phantom/already-shipped entries). **Cluster 128** (`v128.0.0`) hardened A2A delivery (client timeouts; push retry/backoff + metric; SSE error visibility). **Cluster 129** (`v129.0.0`) bounded the MCP streamable buffer + surfaced the outbox quarantine error + converted request-handler `unreachable!()` to typed errors. **Cluster 130** (`v130.0.0`) lifted observability/MCP test coverage (pure-parser extraction). **Cluster 131** (`v131.0.0`) closed delivery-unification as substantially-addressed (declined a risky storage merge). **Cluster 132** (`v132.0.0`) shipped `GET /operator/audit` (global cross-workspace audit, gated by the new `audit:read-global` capability), completing the 127–132 sweep. **UI track:** **Cluster 133** (`v133.0.0`) repaired the broken `/ui` write path (4 undefined JS refs) + added `tests/ui_js_contract.rs` (a CI guard for undefined-helper bugs — the `/ui` JS is otherwise untested, no browser in CI); feature clusters 134+ build on it. **Cluster 134** (`v134.0.0`) added emoji reactions in the `/ui` (over new `/ui/api/messages/:mid/reactions` routes reusing the tested handlers). **Cluster 135** (`v135.0.0`) added message pins in the `/ui` (per-message 📌 toggle over `/ui/api/threads/:tid/pins`). **Cluster 136** (`v136.0.0`) added group DMs in the `/ui` (a new "Group DMs" tab: open/list/read/post over new `/ui/api/.../group-dms` routes reusing the tested `group_dm::*` handlers; the conversation pane reads via the existing thread-messages route). **Cluster 137** (`v137.0.0`) added a deliveries & DLQ operator view in the `/ui` (a new "Operator" tab: list webhook+automation deliveries with status/kind filters + per-row replay over new session-gated `/ui/api/workspaces/:wid/deliveries[/:did/replay]` routes reusing the tested `delivery_ops::*` handlers; works on a plain operator login since list=`workspace:read`/replay=`workspace:write` match the session caps — global audit/reindex controls deferred to 138 since they need elevated bearer tokens). **Cluster 138** (`v138.0.0`) completed the "Operator" tab with the global-audit view (bearer, `audit:read-global`; calls top-level `/operator/audit` directly) + reindex-embeddings controls (workspace reindex on a session via new `/ui/api/operator/reindex-embeddings[/:job_id]` write routes; system-wide reindex needs a `token:admin` bearer). The operator-console arc (137–138) is complete. **Cluster 139** (`v139.0.0`) added 1:1 direct messages in the `/ui` (a new "DMs" tab: open/list/read/post over new `/ui/api/workspaces/:wid/dm` + `/ui/api/dm/:id/messages` routes reusing the tested `dm::*` handlers; the conversation pane reads via the existing thread-messages route — the exact parallel to group DMs 136). **Cluster 140** (`v140.0.0`) added a workspace presence roster in the `/ui` (a new "Presence" tab rendering the realtime `presence_snapshot` frames that already ride the WS subscribe; online/away controls; no backend change — presence is WS-only). Further UI work is open-ended (slash commands remain unsurfaced in the `/ui`). **Cluster 141** (`v141.0.0`) fixed the **published mdBook docs**: the site's sidebar had ~20 dead links because `book/src/SUMMARY.md` referenced the canonical docs via `../docs/*` paths that escape mdBook's `src/` (mdBook silently skips them), so only 3 pages built and every `docs/*` link 404'd. New **`book/sync-docs.sh`** stages the 21 SUMMARY-referenced `docs/*.md` into `book/src/docs/` (generated, gitignored) before `mdbook build` (wired into `docs.yml`), rewriting out-of-`docs/` repo-root links to GitHub URLs and flattening `[[wikilinks]]`; SUMMARY/intro/api dropped the `../`. Added a landing quickstart + custom 404. **NOTE for future docs edits:** new published pages must be added to `book/src/SUMMARY.md` (drop `../`) *and* listed in `book/sync-docs.sh`'s copy set. The `docs` CI job does not fail on dead links (a link-checker is logged in Open Work). **Cluster 142** (`v142.0.0`) added the slash-command registry in the `/ui` (a new "Slash" tab: register/list/revoke over new `/ui/api/workspaces/:wid/slash-commands[/:cid]` routes reusing the tested `slash_commands::*` handlers; one-time webhook secret for `http` handlers; execution stays message-triggered via `/name args`). This surfaced the last unsurfaced backend collaboration feature — the `/ui` now covers the full backend surface. **Cluster 143** (`v143.0.0`) began UI polish (post-parity): richer message rendering — `renderMessages` shows `posted_at` timestamps + an inline slash-command result block from `slash_command`/`slash_response` metadata (completing the slash loop in the thread view). UI-only, no backend. **Cluster 144** (`v144.0.0`) added a **docs dead-link gate** (`book.toml` `[output.linkcheck]` renderer, `warning-policy = error`, `follow-web-links = false`; `docs.yml` installs `mdbook-linkcheck`) so `mdbook build` fails on dead internal links. Turning it on surfaced **35** latent broken published links, all fixed in `book/sync-docs.sh`: space-named files are now staged under **hyphenated** names (`Capability Map.md` → `Capability-Map.md`, etc. — SUMMARY/introduction updated; kills `%20`-in-path) and out-of-set links (unpublished docs/repo files) are GitHub-rewritten. The linkcheck renderer nests HTML under `book/build/html/` — the deploy uploads from there. **NOTE for future docs edits:** the gate now fails the build on any dead internal link, so a new published page must be added to `SUMMARY.md` + `sync-docs.sh`'s copy set (hyphenated path if the filename has spaces), and links to non-published docs must be GitHub-rewritten in `sync-docs.sh`. Also reconciled the backlog docs (audit API shipped 132; 134–143 UI track). **Cluster 145** (`v145.0.0`) began an **MCP streamable spec-completeness arc (145–148)**: 145 landed the JSON-RPC/lifecycle basics — `initialize` protocol-version negotiation (`maidan-mcp` exposes `SUPPORTED_PROTOCOL_VERSIONS`, currently `["2024-11-05"]`), `MCP-Protocol-Version` header validation on `POST /mcp` + `/mcp/streamable`, JSON-RPC batching (array→array) + notifications (`202`) on `POST /mcp`. **Cluster 146** (`v146.0.0`) added `GET /mcp/streamable` (server→client SSE stream, session-aware, reusing the `subscribe_notifications` broadcast) + `Accept`-based JSON/SSE content negotiation on the POST. **Cluster 147** (`v147.0.0`) added resumability: session SSE frames carry a monotonic `id:` + a bounded (256) per-session replay log; `GET /mcp/streamable` with `Last-Event-ID` replays the retained frames, then live. A streamable session now **survives a dropped POST stream** (removed close-on-drop; TTL/DELETE clean up) so reconnect works — and a follow-up POST whose SSE leg is gone degrades to inline JSON (200) instead of 500. **Cluster 148** (`v148.0.0`) concluded the arc: **server→client requests** — `McpServer::request_client(session, method, params)` issues `sampling/createMessage`/`roots/list`/`elicitation/create` over the session SSE, gated on the client's declared capabilities (captured from `initialize` into the session, previously discarded); the client's response is POSTed back and `POST /mcp/streamable` routes a JSON-RPC response (has `id`, no `method`) to the awaiting caller via a per-session `pending` oneshot map. **The MCP streamable spec-completeness backlog item is closed** (145 version/batching/notifications, 146 GET SSE + `Accept`, 147 resumability, 148 bidirectional). Honest note: `request_client` has **no organic in-tree caller** yet — it's transport capability for future features, implemented + tested end-to-end, not stubbed. No open backend capability gaps remain in the backlog. **Cluster 149** (`v149.0.0`) began an **MCP-agent-surface arc** (from next-arc research across UI polish / missing features / token efficiency / `request_client`): 149 added MCP inbox/mention tools (`list_mentions`/`get_inbox`/`mark_inbox_read` in `tools/member.rs`) so an MCP-only agent can discover it was @mentioned (the reads existed in store+HTTP but weren't in the MCP catalog). **Cluster 150** (`v150.0.0`) added thread/member/kind filters to `GET /mcp/stream` (new `channel_id`/`thread_id`/`member_id`/`kinds` query params wired into the existing `EventFilter` in `mcp_stream.rs::resolve_stream_params`; `kinds` comma-separated snake_case via `EventKind::parse`, unknown → 400) — the "await my mention" primitive. The MCP-agent-surface pair (149 discover + 150 await) is complete. **Cluster 151** (`v151.0.0`) shipped the first **token-efficiency lean read**: `get_thread_context` now returns edit records as metadata only (`{id, message_id, editor_id, edited_at}`) by default — the `body_before`/`body_after` copies were the largest token cost in a context pack — with opt-in `include_edits=true` for full bodies (`get_workspace_context` inherits it via its nested packs); also clamped `list_messages` to `1..=500`. **Cluster 152** (`v152.0.0`) brought the same lean-edits default to the **REST** context pack (`GET /threads/:id/context` + `/workspaces/:wid/context`): `ThreadContext.message_edits` is now `Vec<MessageEditView>` (optional `body_before`/`body_after`, omitted unless `include_edits=true` query param), and added `snippet_only=true` to `GET …/search` (drops full `body`; semantic hits — which have an empty snippet — get a UTF-8-safe truncated body prefix via `SearchHit::into_snippet_only`, `SNIPPET_FALLBACK_BYTES=240`). The token-efficiency lane now covers both context-pack surfaces + search. The user asked to run **all three next-arc lanes in order**: (1) token-efficiency ✅ 151+152, (2) live `/ui` thread view ✅ **Cluster 153** (`v153.0.0`) — in `static/index.html`, a WS domain-event frame (`typeof log_id === "number"`) whose `thread_id === selectedThreadId` and whose `kind` ∈ the thread-content set (`message_posted`/`edited`/`tombstoned`, `reaction_added`/`removed`, `message_pinned`/`unpinned`) triggers `scheduleLiveRefresh()` → debounced `loadMessages()` (≤1 reload/300ms) + a `● live` flash; guarded by a new `ui_js_wires_live_thread_refresh` static test. (3) `request_client` — **split into two clusters** because the tool-dispatch path (`handle`→`dispatch`→`tools_call`) does NOT carry the streamable session id, and the session channel is a single-consumer mpsc that only the POST leg drains: **154** ✅ (`v154.0.0`) = the **GET-stream delivery fix**: `request_client` now delivers via a new per-session broadcast (`push_client_request`/`subscribe_client_requests` in `streamable_session.rs`) that `stream_get` merges into the canonical `GET /mcp/streamable` stream — previously it pushed to the POST-leg mpsc, so a GET-only client never saw server→client requests. POST-leg mpsc/replay untouched; server→client requests are now GET-stream-only (spec-canonical). **155** ✅ (`v155.0.0`) = a **real caller**: an optional session id is threaded through `handle`→`handle_in_session`→`dispatch`→`tools_call`→`tools::dispatch` (`handle` delegates `None`; `open_new_streamable_session`/`follow_up_on_open_session` + the POST JSON-accept path pass `Some(session)`), and the sampling-backed **`summarize_thread`** tool (gather thread → `request_client(session, "sampling/createMessage", …)` → return the client's summary) is its first organic caller; e2e `summarize_thread_tool_samples_via_the_client` proves the 154 path end-to-end. **The three-lane next-arc plan is complete.** A 5-agent research sweep then set the **post-v155 program — four arcs, run in order** (user: "do all in that order"): **(1) Enterprise hardening** — **156** ✅ (`v156.0.0`) shipped SIGTERM graceful shutdown (`main.rs`) + default 30 s `statement_timeout` (`config.rs`); **157** ✅ (`v157.0.0`) made `AUTH_DISABLED` fail-closed — needs the explicit `MAIDAN_ALLOW_INSECURE_NO_AUTH` ack (`validate_insecure_no_auth` in `config.rs` + `auth_disabled_from_env` in `auth.rs`), never in prod; ack added to `compose.yaml` ×5 + `helm/maidan/values-ci.yaml` so the required smoke jobs stay green. **158** ✅ (`v158.0.0`) added keyless cosign signing to the container images (`sign-images` job in `release.yml`, by digest; trivy scan deferred to arc 2). **Arc-1 quick-wins done; flagship channel/thread RBAC next** — authz is workspace-flat today (any `message:post` token r/w any channel incl. private; private enforcement only in `subscribe_grants.rs` fan-out, self-asserted). Planned as 3 clusters (full detail persisted in scratchpad `rbac-plan.md`): **159 (A)** ✅ (`v159.0.0`) landed `channel_members` table + store (`postgres/sqlite/channel_members.rs`, mirror `group_dm.rs`) + migration (pg 0032 / sqlite 0031) + `ChannelMember`/`ChannelMemberRole` in models.rs + 4 trait methods, NO enforcement (zero blast radius); **160 (B)** ✅ (`v160.0.0`) landed `ensure_channel_access`/`ensure_thread_access`/`ensure_message_access`/`can_access_channel` in `maidan-auth/src/access.rs` (bypass=skip; public=workspace-open; private=explicit `channel_members`; `__dm__` exempt) enforced on ALL REST content routes (`channel` get/list + create auto-adds creator admin, `thread` all, `message` all, `social` all), `search` (hit-filter), and the workspace-context pack (thread-filter); e2e `channel_access_e2e` proves non-member-denied/creator-allowed/member-allowed/public+DM-unchanged. **161 (C)** ✅ (`v161.0.0`) added MCP point-access enforcement — a pre-dispatch gate in `tools::dispatch` (`enforce_channel_access`) that resolves each content tool's `channel_id`/`thread_id`/`message_id` arg and calls the `ensure_*` helpers (list_threads/list_messages/post_message/get_thread_context/summarize_thread/pins/edit/mention/votes/reactions), plus `resources_read` gating `threads/{id}` + `channels/{id}`; test `mcp_denies_non_members_in_private_channels`. **162 (D)** ✅ (`v162.0.0`) filtered the MCP aggregate reads — `search_messages` (threads store+auth in, drops hits by `can_access_channel`), `list_channels` (auth in, hides private non-member channels), `get_workspace_context` (dispatch-arm filter of `v["threads"]` by channel access) — closing the channel-content vuln on REST + MCP. **163 (E)** ✅ (`v163.0.0`) verified WS/MCP subscribe grants — `apply_subscribe_grants(state, auth, filter)` drops asserted private-channel grants the caller isn't a member of (public/`__dm__` pass; bypass keeps all); `ws.rs` resolves auth before applying grants, `mcp_stream.rs` passes `auth`. Closes the private-channel event leak. **164 (F)** ✅ (`v164.0.0`) added `channel:admin` cap + `/channels/:cid/members` REST (add/list/remove, gated) + MCP `add/list/remove_channel_member` tools + OpenAPI + contracts/matrix + membership e2e — private channels are now operational. **165 (G)** ✅ (`v165.0.0`) guarded `reference.rs` (REST `create`/`list_references` + MCP `add_reference` via `ensure_thread/message_access` on both ref sides — also fixed the missing `ensure_workspace`). **The channel/thread RBAC arc (159–165) is COMPLETE**: enforced on read/write (REST+MCP), events (WS+MCP SSE), management (`channel:admin`), and references. Arc-1 enterprise hardening done; **arc 2 (perf + CI/CD)** began: **166** ✅ (`v166.0.0`) = R3 sqlite pragmas in `after_connect` (per-connection FK/busy_timeout bug — `sqlite_pool_options_with`) + H1 webhook per-workspace fan-out (`list_enabled_webhook_subscriptions_for_workspace`). **167** ✅ (`v167.0.0`) = R2 rate-limiter map eviction (`MEMORY_SWEEP_THRESHOLD` in `rate_limit/limiter.rs`) + H6 `PostgresSearch.model_tables` cache. Remaining perf: H4 outbox `list_pending` JOIN payload + batch `mark_published`, H2 delivery-cursor coalesce, R1 env-tunable `BROADCAST_CAP`. **CI/CD workflow changes (arm64 runner, build-once image, gha cache, trivy) are DEFERRED until GitHub Actions recovers** — they only run in Actions, unvalidatable during the outage. ⚠️ Still admin-merging w/ local validation during the outage; RE-RUN CI on main when GitHub is back (`subscribe_grants.rs` still self-asserts membership — private channel EVENTS still leak to a non-member who asserts grants), `reference.rs` (no ws/access check at all), the DM-via-generic-thread-route read (pre-existing; `__dm__` exemption preserves it — tighten via DM-participant check), and the `channel:admin` + `/channels/:cid/members` management API. **RLS deferred** (needs per-connection GUC refactor on the shared PgPool; app-layer closes the primary vuln). **⚠️ OUTAGE MODE (2026-08-06):** GitHub Actions had a "major" incident (action-download 503s failing job setup on `bootstrap-strip`/`otlp`/most jobs — NOT the code); user authorized **skipping the CI wait** and admin-merging. Clusters from **160** were validated **locally** (fmt + clippy + full `cargo test -p maidan-server` incl. testcontainers via local docker + mdbook linkcheck) and admin-merged without green Actions. **When GitHub recovers, re-run CI on `main` to confirm.** **(2) Perf + CI/CD** — H1 webhook full-table scan→workspace filter, H6 model cache, R3 SQLite pragmas in `after_connect` (correctness bug), H4 outbox JOIN, R2 rate-limiter leak, H2 cursor coalesce; native arm64 release runner (kills ~2h QEMU build), build-smoke-image-once + reuse across the 4 Docker jobs, gha cache; **(3) Agentic features** — structured message content (typed blocks over `body`/`metadata`), MCP structured backpressure (429→typed retry-after in envelope), HITL approvals over the elicitation transport, task assignment/handoff (`assignee`/claim on Thread); **(4) Token round 3** — MCP `search_messages` snippet_only parity, capability-filtered `tools/list` + trimmed catalog descriptions, lean write-acks + omit-empty metadata, opt-in lean event frames. See memory `maidan-next-arc-program`. **NOTE:** adding an MCP tool needs a `tools/*.rs` handler + `tools/mod.rs` dispatch+capability arms + `catalog.rs` schema + BOTH `contracts/mcp-tool-names.json` and `contracts/mcp-capability-map.json` (kept sorted); contract tests enforce the sync. The shipped streamable subset's substantive record is the **Cluster 35.0 retro** (not 78).
- **Gate tags cut (all four):** **`maidan-2.0`** (`v58`), **`maidan-agent-1.0`** (`v76`), **`maidan-operator-1.0`** (`v101`), **`maidan-scale-1.0`** (`v120`).
- **No `v93`–`v100` tags (intentional):** clusters **93–101** shipped as a single batch PR (#264) and were released as **`v101.0.0`** — they were never separate releases, so there are no `v93.0.0`–`v100.0.0` tags to cut. Version tags cut: `v101.0.0`, `v102.0.0`–`v120.0.0`, and `v121.0.0`–`v167.0.0`.
- **CI:** 8 required checks on `main` (incl. `scale-out smoke`, promoted at the scale gate; `promtool (alert rules)` + `otlp smoke`, promoted in Cluster 124).
