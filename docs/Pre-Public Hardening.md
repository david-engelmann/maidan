# Pre-public hardening

**Pickup:** [Handoff.md](Handoff.md) if you are a later agent executing this checklist.

**Audience:** you (maintainer), after the product surface works and before
you invite the world in — blog, Show HN, agent frameworks, recruiters,
other engineers reading the code as a signal about *you*.

**When to use:** Program D / read-replica arc is **closed** (v266). Feature
work on **270–272** is the Claude agent's ladder (269 shipped v269.0.0) — do not duplicate it.
This doc is reputation: cleanup, evidence, presentation. Polish, not
features. Expansion lives in [Expansion Bets.md](Expansion%20Bets.md).

**Non-goals:** product gaps in [Open Work.md](Open%20Work.md) /
[Remaining Work.md](Remaining%20Work.md) (DAG follow-ups, Slack UX
polish). Clusters **265–266** (read-replica) are **SHIPPED**. Federation
egress (`content → parts`) is **SHIPPED** at v267. Do not list those as
product-track future. Workspace import **store** shipped (269); **270** REST + search token-aware
replica routing (271–272) are the other agent's optional-deferrals sweep —
not this checklist. This doc is about making what you *already built*
look and feel finished.

**Companion sources (evidence gathered 2026-08-25 re-scan):** repo scan of
`crates/`, `docs/`, `.github/`, `CLAUDE.md`, `CONTRIBUTING.md`,
`SECURITY.md`, `deny.toml`, `book/src/SUMMARY.md`. Re-run greps when you
start a workstream — numbers below are snapshots, not eternal truth.

---

## Strengths to preserve (do not "clean" these away)

- Dual-backend discipline (Postgres + SQLite) with parity tests and
  contracts under `contracts/`.
- Capability-scoped auth enforced in CI (HTTP + MCP maps).
  `token:admin` exists (`TOKEN_ADMIN` in `maidan-auth`); keep it off
  default agent tokens.
- Eight required CI checks that actually mean something (lint/deny,
  secrets, unit, integration, compose smoke, scale-out smoke, promtool,
  OTLP smoke).
- Compile-time bootstrap strip, fail-closed `AUTH_DISABLED`, cosign on
  releases, threat model, Integration.md + mdBook for strangers.
- Almost no `todo!()` / `unimplemented!()` / `FIXME` left in production
  Rust — the unfinished feel is *narrative residue*, not stub code.
- 13 crates with a clean split (`maidan-a2a` … `maidan-types`). No
  accidental Slack/ACP crate to "finish."

---

## Severity legend

| Sev | Meaning |
|-----|---------|
| **P0** | Someone cloning cold will distrust the project or misconfigure prod within an hour |
| **P1** | A careful reader will mark you as "shipped fast, didn't finish the room" |
| **P2** | Polish; do after P0/P1, still worth sequencing |

---

## Findings snapshot (evidence)

### Archaeology / narrative residue — **P1**

- **~771** matches for `Cluster N` / `PR #N` style breadcrumbs inside
  `crates/` alone (doc comments, module banners, inline history; was
  ~754 on 8/24). Hot spots: `crates/maidan-types/src/models.rs` (**33**
  Cluster refs), `events.rs`, plus server/store/mcp modules that narrate
  which cluster invented a field.
- Public crate docs (`maidan-types`) expose **delivery history**
  ("Cluster 237, Program C") instead of **domain meaning**. That reads
  as a private diary to an outsider.
- Maintainer vault docs still dominate mental weight: `CHANGELOG.md`
  ~3.8k lines, `docs/Capabilities.md` ~2k, `docs/Roadmap.md` ~500,
  `docs/Open Work.md` is a single mega-paragraph of program history.
  Integrators are correctly pointed at Integration.md, but the default
  GitHub browse lands on density.
- Obsidian `[[wikilinks]]` still appear in many non-Clusters docs
  (`Open Work`, `Remaining Work`, `Roadmap`, `Architecture`,
  `Integration`, `Production`, `AGENTS.md`). GitHub renders them as
  dead text. This file used to `[[Open Work]]` itself; it now uses
  Markdown links.
- `CONTRIBUTING.md` / `SECURITY.md` open with **"Maidan is pre-release"**
  while four product gates are tagged — mixed signal for a public launch.
- Workspace `Cargo.toml` is `version = "0.0.0"` and `publish = false`
  while git tags are `v269.0.0`. Version story is tag-only; crates.io/SDK
  cannot depend. Pair with the pre-release tone fix.
- Stale crate `lib.rs` banners still narrate the future as past: artifacts
  "S3 arrives Cluster E", search "pgvector arrives Cluster C", store
  "Cluster A CRUD", `mail.rs` "not wired" (it is).

### Bug sweep (2026-08-25 afternoon) — **P0/P1**

Read-only `rg` of `crates/` on `feat/cluster-272-search-replica-metric`
(did **not** edit 272 files). Almost no `todo!` / `FIXME` / `unsafe`
outside sqlite-vec init. The unfinished feel is **lies and one panic**,
not stub code. Details and slices: section **K**.

- **Lie:** `mail.rs` module docs still say "Not wired into the notification
  router yet." Router `notify` spawns `deliver_notification_email` (249),
  with presence skip (253) and digest mode (255). **A6 / K1.**
- **Lie:** [Open Work.md](Open%20Work.md) baseline `v143` and still lists
  generic-thread DM as "**next: Cluster 180**". Code has
  `ensure_thread_access` / `ensure_dm_participant` on `GET /threads/:id`.
  **K2 / E5.**
- **Lie:** Production.md "**Not covered:** load/throughput benchmarking"
  while `scripts/loadgen.sh` + `#[ignore] load_baseline` exist. **H5.**
- **Lie-by-omission:** root README sells "MCP JSON-RPC + streamable HTTP"
  with no `2024-11-05`. **C5 / J2.**
- **Request-path panic:** `AppState::subscribe_resume_secret` panics if
  neither OIDC session secret nor `subscribe_resume_secret` is set.
  `main` always sets one (or refuses boot); `for_tests` sets the test
  constant. Still a landmine for any future constructor. **D3 / K3.**
- **AUTH_DISABLED landmine:** missing `MAIDAN_SESSION_SECRET` falls back
  to `TEST_SUBSCRIBE_RESUME_SECRET` (`b"test-subscribe-resume-secret-32b!!"`)
  with a warn. Fine for tests; a prod misconfig with `AUTH_DISABLED` +
  the insecure ack would mint forgeable resume tokens. **K4 / F4.**
- **hash-v1 default** logs `embedding provider configured` with no
  "this is not semantic" warning. **E4 / K5.**
- **Dead code:** `oidc/member.rs` `member_kind_is_human` is
  `#[allow(dead_code)]` and has **zero** call sites. **K6.**
- **OpenAPI phantoms:** `openapi/paths/` is utoipa-only (`allow(dead_code)`);
  `extensions.rs` still titled "missing from api.rs (Cluster 77)" while
  import/export/purge are real routes. Drift risk, not runtime dead.
  **C3 / K7.**
- **Outbox relay** `list_pending` is `SELECT … ORDER BY id LIMIT n` with
  **no** `FOR UPDATE SKIP LOCKED`. Concurrent replicas can double-publish
  (mark_published is the only fence). Claim paths for threads/schedules
  already use SKIP LOCKED. **K8** (behavior; not Redis).
- **Swallowed cursor:** `event_stream` replay `let _ = store.advance_delivery_cursor`
  — a failed advance looks like success; at-least-once consumers can
  replay or skip. Log it. **K9.**
- **Not bugs:** HMAC `unreachable!` (SHA-256 accepts any key); federation
  `panic!` is inside `#[cfg(test)]`; `let _ = mcp.handle` on notifications
  is JSON-RPC spec (no id → no body); sqlite-vec `unsafe` is the
  documented C init; Redis exists **only** for optional rate-limit
  (`MAIDAN_RATE_LIMIT_REDIS_URL`), not a bus — do not rip it out under
  Hardening H's "no Redis."

### Structure / maintainability — **P1**

Monster modules (`wc -l`, 2026-08-25):

| Lines | Path |
|------:|------|
| 2230 | `crates/maidan-mcp/src/server.rs` |
| 1695 | `crates/maidan-store/src/postgres/mod.rs` |
| 1532 | `crates/maidan-types/src/models.rs` |
| 1455 | `crates/maidan-store/src/sqlite/mod.rs` |
| 1159 | `crates/maidan-store/tests/event_log.rs` |
| 1057 | `crates/maidan-store/src/store.rs` |
| 1037 | `crates/maidan-server/tests/ws_subscribe_e2e.rs` |
| 989 | `crates/maidan-mcp/src/tools/catalog.rs` |
| 961 | `crates/maidan-server/tests/mcp_streamable_e2e.rs` |
| 844 | `crates/maidan-server/src/openapi/paths/api.rs` |

Postgres vs SQLite file trees are nearly parity; intentional deltas:
`postgres/replication.rs` only, `sqlite/pragmas.rs` only. Good — keep
that explicit in Architecture.

`#[allow(clippy::too_many_arguments)]` on hot constructors
(`event_stream`, `state`, `notification_router`) and a few `dead_code`
allows in OIDC / test helpers — smell of "grow the struct instead of
grouping config."

### Stale comments / protocol honesty — **P0/P1**

- **`mail.rs` module docs are a lie.**
  `crates/maidan-server/src/mail.rs` still says "Not wired into the
  notification router yet." It **is** wired (Cluster 249):
  `notification_router.rs` `tokio::spawn`s `deliver_notification_email`
  (best-effort, never retried; durable queue is a follow-up). Digest
  mode (255) and presence skip (253) already exist. Fix the module
  docs in P0. Retry/DLQ is Expansion Bet 4, not this checklist.
- **MCP protocol frozen at 2024-11-05**
  (`SUPPORTED_PROTOCOL_VERSIONS = ["2024-11-05"]` in
  `maidan-mcp/src/server.rs` ~line 30) while **2026-07-28** is what
  current IDE clients may speak. `GET /mcp/streamable` +
  `Mcp-Session-Id` are still first-class; the 2026-07-28 spec removed
  GET stream + protocol-level sessions. This is polish **and**
  Expansion Bet 2 shared risk: do not ship Cursor/Claude deeplinks that
  imply the new rev until the server negotiates it. Document the freeze
  (this doc / README) even if Bet 2 owns the pack.

### Testing & evidence — **P1** (confidence), **P2** (coverage number)

- Coverage floor is **40%** lines (`COVERAGE_MIN_LINES` in
  `.github/workflows/ci.yml`). Fine as a regression floor; weak as a
  public quality claim. Do not raise the number blindly — raise
  *meaningful* coverage on authz, delivery, and store dual-write paths
  first, then consider 50–60%.
- `#[ignore]`d soaks exist and are honest tools (loadgen, chaos,
  replication, read_routing). Public story needs a short
  **how operators run them** page with expected output — otherwise they
  look like abandoned tests.
- ~4.8k `unwrap`/`expect` across crates; ~386 outside obvious
  `/tests/` paths — many are inline `#[cfg(test)]` modules (64 files
  with cfg-test). Still worth a pass that classifies true production
  panics vs test helpers.
- Production `panic!` / `unreachable!` sites are mostly HMAC/"must be
  configured" paths (`session/cookie`, `subscribe_resume`, `webhooks`,
  `state.rs` subscribe secret). Audit each: fail boot vs panic mid-request.
- Ten `ui_*` test files. `ui_js_contract.rs` is **static** analysis
  (bare JS calls resolve). No Playwright, no headless browser job.
  That is correct while `/ui` is an operator console — do not turn
  browser e2e into a polish item.

### Docs & first impression — **P0/P1**

- **No `examples/` tree.** Contracts + Integration.md exist; a cold
  agent engineer still wants copy-paste MCP + REST golden paths in-repo.
  No `mcp.json` snippets either. Overlaps Expansion Bet 2; the
  Hardening piece is "a stranger can copy a file," not a hero DAG.
- README first command is still
  `DATABASE_URL=sqlite::memory: cargo run --bin maidan-server`.
  `docker compose --profile full` is later. Cold clones who do not
  have a Rust toolchain bounce. E2 is docker-or-binary *before* cargo.
- mdBook SUMMARY correctly separates Integrate / Reference / Design /
  Historical — keep that. Do **not** publish Clusters/Retros as the
  front door.
- Gate evidence docs exist (`docs/Gates/maidan-scale-1.0.md`). Extend
  that pattern: one page that points at CI job names + scripts that
  prove the claims you will make in the blog.
- `CLAUDE.md` is an excellent operating manual for agents editing the
  repo; for public humans it is long and cluster-centric. Keep it, but
  make README → Integration the human front door (already mostly true).

### Supply chain & security presentation — **P1**

- `deny.toml` ignores several RUSTSEC ids (incl. `RUSTSEC-2023-0071` via
  openidconnect v4). Document the **public** justification in
  Dependencies.md / SECURITY (already partially there) and track
  "clears on openidconnect v5" as a dated follow-up so it does not look
  like swept under the rug.
- Confirm release artifacts story (cosign bundles, SBOM, ARM64) is one
  click from README Releases blurb — blog readers will check.

---

## Workstreams (checkboxes)

Execute as small PRs. Prefer squash-merge cluster-style only if you want
history; for polish, `chore/` and `docs/` PRs with clear titles are fine.

Do **not** mix these with 269–272. P0 first-impression items (E2, E4,
A5, `mail.rs` module docs) **can** land in parallel with that sweep.

### A. Residue removal — "engineering diary → product docs" (**P1**)

- [ ] **A1. Public API docs scrub (types + events)**
  In `maidan-types` (and any `pub` re-exports), rewrite doc comments to
  describe behavior/invariants. Move "added in Cluster N" to CHANGELOG
  / Capabilities only. Target: `models.rs`, `events.rs`, `usage.rs`,
  `erase.rs`, `lsn.rs`, purge helpers.
- [ ] **A2. Implementation comment scrub**
  Grep `crates/` for `Cluster `, `PR #`, `Program [A-D]`, `Arc [A-Z]`.
  Keep comments that explain *why* a subtle invariant exists; delete or
  rewrite ones that only record delivery chronology. Current count ~771.
- [ ] **A3. Vault vs public split (policy + light moves)**
  Decide and write it down in `docs/README.md`:
  - **Public contract:** Integration, Capability Map, Production,
    Deploy, Threat Model, Architecture, Decisions, Gates, this file,
    Expansion Bets, Path to Impressive, Providers, Protocols, Handoff, Launch.
  - **Maintainer archive:** `Clusters/`, `Retros/`, Roadmap history,
    Open Work mega-log, Post-1.0 tracks.
  Optional: add `docs/archive/` or a top-of-file banner on Open Work /
  Remaining Work: "maintainer planning; not the product contract."
- [ ] **A4. Wikilink pass on public docs**
  Replace `[[...]]` with relative Markdown links in anything linked from
  mdBook SUMMARY Integrate/Reference (and Architecture/Decisions if
  published). Leave Clusters/Retros alone if they stay vault-only.
  Confirmed 2026-08-25: **Production.md** still has `[[Agent Integration]]`
  and `[[Production#…]]`. Integration.md only *mentions* wikilinks.
  Open Work / Remaining Work are vault-archive (A3) unless you publish them.
- [ ] **A5. Tone pass on CONTRIBUTING + SECURITY**
  Replace blanket "pre-release" with accurate maturity language
  (e.g. "stable API surface under the tagged gates; solo-maintained;
  expect rapid post-gate hardening"). Align README status section.
- **A6. Fix the `mail.rs` module-doc lie — ✅ done (Cluster 316)**
  Rewritten: config-gated SMTP; wired from `notification_router`
  (249); best-effort spawn; presence skip (253) + digest (255) applied
  before send; durable retry/DLQ via the mail outbox (305–306).

### B. Structural refactors — "readable modules" (**P1**, no behavior change)

Do **not** mix refactors with feature clusters. One module family per PR.
Do **not** interleave with 269–272 import/search PRs.

- [ ] **B1. Split `maidan-mcp/src/server.rs`**
  Extract: session/lifecycle, tools dispatch, resources/prompts,
  streamable transport glue. Goal: <500 lines/file, clear `mod` tree.
  2230 lines today.
- [ ] **B2. Thin `postgres/mod.rs` + `sqlite/mod.rs`**
  Move read-routing / pool selection / re-exports; keep domain files
  (`threads`, `messages`, …) as the meat. Document the replication-only
  / pragmas-only deltas next to Architecture. 1695 / 1455 lines today.
- [ ] **B3. Split `maidan-types` models**
  Group: identity/workspace, messaging, tasks/DAG/schedules, notifications,
  artifacts/search. `models.rs` at 1532 is a review tax.
- [ ] **B4. Config objects instead of `too_many_arguments`**
  Replace allows on `event_stream` / `state` / `notification_router`
  constructors with typed config/context structs.
- [ ] **B5. Store trait readability**
  `store.rs` 1057 lines — consider subdomain traits (`ThreadStore`,
  `NotificationStore`, …) composed into `dyn Store` or a struct of
  Arcs, *only* if it reduces duplication without a six-month rewrite.
  Spike first; do not boil the ocean.

### C. API / contract consistency (**P1**)

- [ ] **C1. Error shape audit**
  One page (or ADR): REST error JSON, MCP JSON-RPC errors, A2A errors —
  status codes, capability failures, not-found vs forbidden (esp. after
  channel RBAC). Add contract tests where missing.
- [ ] **C2. Naming drift pass**
  Inventory MCP tool names vs REST paths vs event kinds
  (`contracts/*.json` is the source of truth). Public names:
  `claim_next_thread` (not `claim_next`); EventKind
  `MessagePosted` / `ThreadResultSet` / `MentionRecorded` (wire
  `message_posted` / `thread_result_set` / `mention_recorded`).
  Fix only *user-visible* inconsistencies; document intentional
  asymmetries (store helper `threads::claim_next` is internal).
- [ ] **C3. OpenAPI / MCP reference freshness**
  Confirm `gen-mcp-reference` + OpenAPI are required in CI (or
  clearly generated on release) so the published site cannot drift.
  `openapi/paths/api.rs` is 844 lines — freeze a 7-method subset
  before any SDK (Expansion Bet 3); this item is "the spec matches
  the binary."
- [ ] **C4. Deprecation policy**
  Short ADR: how you rename/remove a tool or field post-public (window,
  changelog section, capability bit).
- [ ] **C5. MCP version honesty (until J3) then 2026 copy**
  Until J3: README + Integration say **today `2024-11-05`, 2026-07-28 upgrade is required**.
  After J3: they say **`2026-07-28`**. Never imply 2026 Streamable HTTP while
  `SUPPORTED_PROTOCOL_VERSIONS` is 2024-only. Pack/deeplinks wait on J3.
  Full track: [Protocols.md](Protocols.md) § Required protocol upgrades.

### D. Testing, CI, and runnable evidence (**P0** for claims you will publish)

- [ ] **D1. Evidence index**
  New short doc `docs/Evidence.md` (or expand Gates): for each blog/README
  claim ("multi-replica presence", "at-least-once opt-in", "capability
  matrix", "read-your-writes tokens"), link the CI job and/or
  `scripts/*.sh` + `#[ignore]` test that proves it.
- [ ] **D2. Ignored-test operator guide**
  Document: `scripts/loadgen.sh`, `scripts/chaos.sh`,
  `scripts/replica-harness.sh`, how to run `--ignored` tests, what
  "pass" looks like. One section under Production or Evidence.
- [ ] **D3. Production panic audit**
  Classify every non-test `panic!`/`unreachable!`/`expect` on request
  paths. Prefer `500` + log or fail-fast at boot.
  **2026-08-25 sweep:** only production `panic!` is
  `state.rs` `subscribe_resume_secret` (K3). Other `panic!`/`expect` hits
  are `#[cfg(test)]` or benches. HMAC `unreachable!` in cookie/webhook/
  resume sign is acceptable.
- [ ] **D4. Coverage with intent**
  Pick 3–5 critical modules (channel access, DM participation,
  outbox/`*_with_event`, notification router, consistency middleware).
  Add tests until those are strong; *then* consider raising
  `COVERAGE_MIN_LINES`.
- [ ] **D5. Backend parity ritual**
  Make `backend_parity` / `dialect_parity` visibility obvious in CI
  summary or Evidence.md so dual-backend is not tribal knowledge.
- [ ] **D6. Flake budget**
  Note known timing-sensitive tests; quarantine or rewrite before
  public contributors hit them.

### E. Documentation strangers will actually use (**P0**)

- [ ] **E1. `examples/` directory**
  Minimum set:
  - `examples/quickstart-sqlite.sh` (health + workspace + message + context)
  - `examples/mcp-cursor.json` (or fragment) pointing at `/mcp`,
    protocol `2024-11-05` (see C5 / Bet 2 M.0)
  - `examples/agent-handoff.md` — mention → `wait_for_mention` /
    `claim_next_thread` → `set_thread_result` narrative with curl/MCP
  Keep them CI-smokeable where practical (`bash -n` + a compose profile).
  Hero DAG seed is Expansion Bet 2 M.2; this item is copy-paste paths.
- [ ] **E2. README first screen**
  30-second pitch, one command (**docker-or-binary before `cargo run`**),
  badges (CI, license, release), links to Integration + mdBook +
  Releases. Move cluster mythology below the fold or out.
- [ ] **E3. Architecture diagram**
  One Mermaid (or SVG) in Architecture.md: clients → server → store/bus
  → object store. Blog will steal this; make it accurate.
- [ ] **E4. "What Maidan is not"**
  Short section (README or Integration): not Slack-complete, not a
  model host, not multi-region active-active. Sets expectations; protects
  reputation.
- [ ] **E5. Reconcile stale planning docs**
  Either update `Remaining Work.md` / `Post-1.0.md` headers to current
  tag reality or stamp **"frozen archive as of vNNN"**. Stale "active"
  language is worse than an old archive. Do not resurrect 265–266 /
  federation egress as open product work.

### F. Supply chain & security presentation (**P1**)

- [ ] **F1. Advisory ignore table**
  Human-readable table: id → why ignored → clear condition. Link from
  SECURITY.md.
- [ ] **F2. Release verification snippet**
  Copy-paste: how to verify cosign bundle + SBOM for a release asset.
- [ ] **F3. Threat model vs shipped controls**
  Quick matrix pass: every Threat-Model control cites the code/CI
  evidence (or is marked aspirational).
- **F4. Default-secure demo — ✅ done (Cluster 313)**
  `compose.quickstart.yaml` now runs auth ON; the README happy path mints a
  bearer token with `maidan init` (bundled in the quickstart image, bumped to
  `v312.0.0`), and `scripts/quickstart-two-agents.sh` authenticates with it.
  `AUTH_DISABLED` moved to a clearly-marked "explore without a token (local
  only)" appendix backed by `compose.quickstart.insecure.yaml`. Both paths
  validated end-to-end against a local server; CI validates both compose files.

### G. Repo hygiene & presentation (**P2**, still do before the blog)

- [ ] **G1. Root clutter**
  Confirm `.qodo`, local override files, editor junk are gitignored;
  `compose.override.yaml` should not surprise contributors.
- [ ] **G2. LICENSE/copyright headers policy**
  Decide whether crate roots need license blurb; be consistent.
- [ ] **G3. Issue/PR templates for strangers**
  Bug / question / security pointer that do not assume cluster jargon.
- [ ] **G4. CODEOWNERS / support expectations**
  Solo maintainer: say response norms so silence is not read as abandonware.
- [ ] **G5. Changelog for humans**
  Keep the giant CHANGELOG, but maintain a short `CHANGELOG-highlights.md`
  or GitHub Release body template for the last gate + last 10 tags.

### H. Performance, load testing, and optimization (**P1**)

This is a **code-improvement** track, not an expansion bet. Do not
confuse it with Program D (closed at v266) or with 271–272 search
replica routing (other agent). The substrate already has harnesses;
what is missing is *using* them on the agent-shaped hot path, recording
budgets, and only optimizing what the numbers move.

**What already exists (do not rebuild):**

| Piece | Where | What it actually measures |
|-------|--------|---------------------------|
| Load/soak harness | `scripts/loadgen.sh` + `#[ignore]` `load_baseline` in `crates/maidan-server/tests/loadgen.rs` (Cluster **198**) | Concurrent REST **post message / read thread / search**. Reports min/mean/p50/p95/p99/max + throughput. Default: in-process **SQLite**, 8 workers x 50 iters. Can point at a live URL. **Not a CI gate** (hardware flake). Percentile math *does* run in CI. |
| Search microbench | `cargo bench -p maidan-search --bench search_hot` + `benches/SEARCH_BASELINE.md` (Cluster **109**) | SQLite FTS5 + brute-force cosine on **200** in-memory messages. CI-friendly floor, not a Postgres/HNSW SLA. |
| Store microbench | `cargo bench -p maidan-store --bench store_hot` + `benches/STORE_BASELINE.md` (Cluster **120** / `maidan-scale-1.0`) | SQLite `list_members` (32). Same caveat. |
| Scale-out smoke | required CI job + `scripts/replica-harness.sh` | Correctness under replica/compose, **not** throughput. |
| Metrics / OTLP | `/metrics`, Cluster 123 OTLP smoke | HTTP latency histograms exist. **MCP per-tool latency is not exported** (Production.md). |

**What is a lie / hole:**

- `docs/Production.md` still says load/throughput benchmarking is
  "**Not covered** (bench harness, Cluster 109)". Cluster **198**
  shipped `loadgen`. Fix that sentence in H5.
- Default `loadgen.sh` never touches **MCP, WebSocket, `claim_next_thread`,
  DAG waits, or artifact upload** — the agent-shaped mix.
- Default target is SQLite in-process. A public scale claim needs a
  **Postgres + (optional) replica** soak with the numbers checked in
  (machine-tagged, not an SLA on GitHub runners).
- Criterion benches are tiny N. They catch regressions in the floor,
  not "10k agents in a workspace."
- Known leftover **measured** optimizations from Open Work (only do
  these after a before/after loadgen run):
  1. Workspace-context is **build-then-RBAC-filter**; filter-before-build
     is the real win (pagination-sensitive).
  2. Search deny-set is `list_channels` + per-channel membership; a
     single "my private channels" query would be cheaper.
  3. Full DM-at-query-level for search (eliminating post-filter).
- **Declined, do not reopen:** batched `pg_notify` (delivery-core risk,
  Open Work). Redis. External vector DBs for vanity benches. SPA `/ui`
  rewrite for speed.

**Slices (cluster-sized, evidence-first):**

| ID | Scope |
|----|--------|
| **H1** | Refresh baselines: run `scripts/loadgen.sh` on SQLite *and* `docker compose --profile full` Postgres. Check results into `docs/Evidence.md` or a `benches/LOADGEN_BASELINE.md` with hardware tag, date, concurrency, mix. Re-run search/store criterion; update SEARCH/STORE_BASELINE.md if they drifted. |
| **H2** | Agent-shaped mix in loadgen: add ops for `claim_next_thread`, MCP `post_message`/`wait_for_ready` (or REST equivalents), WS subscribe lag, optional artifact PUT. Keep REST post/read/search. Still `#[ignore]`d. |
| **H3** | Optional **nightly** (not required CI): Postgres soak 60s, concurrency 32, fail only on error rate, never on p99. Required CI stays flake-free. |
| **H4** | Optimizations **only with H1 numbers**. First candidates: context filter-before-build, search deny-set query, then stop. No Redis. |
| **H5** | Production.md: strike "Not covered: load/throughput"; document `loadgen.sh`, what it measures, that it is not an SLO gate. Add MCP per-tool latency histogram if you are about to claim MCP is the hot path (otherwise skip). |
| **H6** | Scale-1.0 gate budget vs today's numbers. If `docs/Gates/maidan-scale-1.0.md` still holds, say so. If not, amend the gate rather than quietly rotting. |

**Order relative to other work:** H1 + H5 are parallel with Hardening P0
and with 270–272 (read-only measurement). H2–H4 wait until you have a
checked-in Postgres baseline. Do not start H4 during the other agent's
import/search PRs (same hot files).

### I. Provider matrix (usable with what people already run) (**P1**)

"Support whatever database the user is comfortable with" does **not**
mean a third SQL dialect. The `Store` trait is **228 methods**; Postgres
and SQLite are already write-twice (~18k src lines each). Adding MySQL /
MariaDB / Mongo / Dynamo is a multi-quarter fork of store + search +
bus + replicas, for hosts that will not give you `LISTEN/NOTIFY`,
`pgvector`, WAL LSNs, or FTS5.

The usable reading: **two dialects, many hosts**, plus the other
pluggable surfaces (embeddings, object store, IdP, mail). Prove those
hosts. Do not grow the trait.

**What is already pluggable (code, 2026-08-25):**

| Surface | Trait / switch | Implementations today | How users actually vary |
|---------|----------------|----------------------|-------------------------|
| Database | `DATABASE_URL` → `PostgresStore` / `SqliteStore` | Postgres (+ optional streaming replica) and SQLite | Postgres-compatible *hosts*: RDS, Aurora, Cloud SQL, Neon, Supabase, Crunchy, AlloyDB. SQLite file / `:memory:` / Pi. Replication, `pgvector`, LISTEN bus are **Postgres-only** (intentional: `postgres/replication.rs` vs `sqlite/pragmas.rs`). |
| Search | `Search` trait | `PostgresSearch` (`tsvector` + `pgvector`) / `SqliteSearch` (FTS5 + optional `sqlite-vec`) | Same as DB. Semantic quality is the **embedding provider**, not a vector SaaS. |
| Embeddings | `EmbeddingProvider` | `hash-v1` (offline fake) and `openai-compatible` | One HTTP shape covers OpenAI, Azure OpenAI, vLLM, TEI, Ollama, many Voyage/others that speak `/v1/embeddings`. **No** native Anthropic/Voyage SDK. Chat LLMs are **not** in Maidan (agents bring the model; MCP sampling is the client). |
| Artifacts | `ArtifactStore` | `LocalFsStore` and `S3Store` (MinIO / AWS S3) | Any **S3-compatible** endpoint (R2, B2, Garage, Seaweed, GCS XML API). Native GCS/Azure blob are not implemented. |
| Event bus | `EventBus` | `InMemoryBus` (SQLite/dev) and `PostgresBus` (`LISTEN/NOTIFY`) | Multi-process requires Postgres. SQLite is single-process. No Redis bus (Hardening H). |
| Human auth | OIDC discovery | Generic `openidconnect` + mock | Any OIDC IdP (Keycloak, Auth0, Google, Okta, Authentik). **No SAML/SCIM** (document the requirement). |
| Mail | `MailTransport` | SMTP via lettre only | SES / SendGrid / Mailgun / Postfix as **SMTP relays**. No native HTTP mail API. |
| Agent runtime | none | none | Correct. Maidan is substrate. |

**Lock-in that is real:**

- SQLite cannot grow a multi-replica LISTEN bus or HNSW. Users who want
  HA pick Postgres (or a Postgres-compatible host), not "SQLite at
  scale."
- `hash-v1` default is **not** semantic. Prod must set
  `MAIDAN_EMBEDDING_PROVIDER=openai-compatible` or they will think
  search is broken.
- S3 env names are AWS-shaped (`S3_ENDPOINT`, `S3_ACCESS_KEY_ID`). R2/MinIO
  work if you point the endpoint; this is a docs problem more than code.
- OIDC is untested-as-matrix: discovery *should* work; we do not CI
  Keycloak vs Google.

**Slices:**

| ID | Scope |
|----|--------|
| **I1** | `docs/Providers.md` — **written 2026-08-25.** Keep true when env vars change. |
| **I2** | Embedding matrix: CI mock of openai-compatible (already have HTTP shape) + a compose profile or script that runs Ollama *or* TEI optionally. Document Voyage/Azure as "if they speak `/embeddings`." Do **not** add a second embedding protocol. |
| **I3** | Object-store recipes: MinIO (compose already), Cloudflare R2, AWS S3. Same `S3Store`. Native GCS/Azure only if a user is blocked on S3 interop. |
| **I4** | OIDC recipes: Keycloak (self-host) + one SaaS (Google or Auth0). Mock stays for tests. SAML stays out. |
| **I5** | Postgres-compatible host smoke: one CI or runbook against a Neon-like URL (or Testcontainers vanilla Postgres, which we already have) plus a **written** "Aurora/RDS/Supabase: use `DATABASE_URL`, enable `pgvector`, do not need a Maidan fork." |
| **I6** | SQLite edge: confirm `sqlite-vec` feature story in Providers.md. **Spike only** (do not commit a third backend): does a LibSQL/Turso URL work as SQLite today, or does sqlx reject it? Write the answer; implement only if it is a URL/driver flag, not a Store rewrite. |

**Do not:** MySQL, MariaDB, MongoDB, Dynamo, Cockroach-as-a-new-dialect
(Cockroach PG wire is I5 documentation or a no), native Pinecone/Qdrant,
native Anthropic embeddings SDK, embedding an LLM in-process.

### J. Integration protocols (plug into the stack they already speak) (**P0** for J3)

"Support whatever protocol people need" does **not** mean a fourth agent
protocol. 2026 industry stack (AAIF / Linux Foundation): **MCP** = agent↔tools,
**A2A** = agent↔agent, REST/OpenAPI + WS + webhooks = existing IT, **AG-UI**
= optional frontend. IBM ACP merged into A2A (2025-08-29). Zed ACP is a
different job (editor↔coding agent). Maidan already speaks the four
transports.

**MCP `2024-11-05`-only is not acceptable.** Current spec is `2026-07-28`
(stateless Streamable HTTP, `Mcp-Method`/`Mcp-Name`). J3 is a required
upgrade cluster, not a freeze-on-2024 decision. J2 is only so we do not
lie *until* J3 lands. See [Protocols.md](Protocols.md) § Required protocol
upgrades.

**What is already on the wire (code, 2026-08-25):**

| Surface | Today | Honest caveat |
|---------|-------|----------------|
| REST + OpenAPI 3.0 | `GET /openapi.json` | No `workspaces.list`. Hero seed is REST/CLI. |
| MCP | `POST /mcp`, streamable, SSE, `maidan mcp-stdio` | **`2024-11-05` only.** Streamable still uses `Mcp-Session-Id` + GET. Spec `2026-07-28` is stateless and requires `Mcp-Method`/`Mcp-Name`. |
| WebSocket | `GET /ws/subscribe` | Resumable. Agent↔UI live path. |
| A2A JSON-RPC v1.0 | `POST /a2a/v1/rpc` | Subset of methods. Egress parts **text-only** (v267). No gRPC. |
| A2A Agent Card | `/.well-known/agent-card.json` | Custom schema, not spec v1.0 `supportedInterfaces[]`. |
| Webhooks / slash / FSM hooks | HTTP callbacks | The n8n/Zapier path. |
| OIDC + app OAuth + Prometheus | humans / apps / scrape | Agents stay on capability bearers. Not MCP resource-server OAuth yet. |

Operator page: [Protocols.md](Protocols.md). Do not duplicate Slack projector
(Bet 1), MCP `examples/` pack (Bet 2), or the TS SDK (Bet 3) here.

**Slices:**

| ID | Scope |
|----|--------|
| **J1** | `docs/Protocols.md` — **written 2026-08-25.** Keep true when routes or protocol versions change. |
| **J2** | **Holding pattern only.** README/Integration: today `2024-11-05`, **upgrade to 2026-07-28 required (J3)**. No 2026 deeplinks until J3 is green. Not a decision to stay on 2024. Shared with C5. |
| **J3** | **Required MCP `2026-07-28` upgrade** (P0 cluster; this *is* Bet 2 M.0). Negotiate 2026 as current. `Mcp-Method`/`Mcp-Name` headers. Stateless: 2026 clients must not need `Mcp-Session-Id`. Rehome GET-session live-wait onto `/mcp/stream` / WS / `wait_for_*` — do not call that 2026 Streamable HTTP. Tests + then advertise 2026. Optional one-release 2024 *client* fallback if it does not restore the session lie. **Do not ship a public cut or MCP pack on 2024-only.** |
| **J4** | Align Agent Card with A2A v1.0 `supportedInterfaces` (`JSONRPC` only). Do not add gRPC to fill the array. |
| **J5** | A2A file/data parts on egress (267 was text). Artifact-backed round-trip. |
| **J6** | Spike MCP OAuth resource-server (RFC 8707) only if a real Cursor/Claude remote host refuses bearer tokens. Do not replace capability ACL. |
| **J7** | One n8n/Zapier recipe: signed webhook + REST post. Docs, not a new transport. |
| **J8** | LangGraph / CrewAI / Agents SDK recipe on REST+WS or MCP tools. No in-process runtime. |

**Do not:** a Maidan-native agent protocol; IBM ACP; Zed ACP as workspace;
native AG-UI/CopilotKit this quarter; A2A gRPC "for completeness"; GraphQL;
gRPC for `/workspaces`; ANP/AP2/A2UI/MCP Apps as required; MCP create-*
bootstrap tools; OpenAI Assistants as a native wire.

### K. Bug sweep leftovers (fix in polish PRs, not 272) (**P0/P1**)

Full `rg` 2026-08-25. Do **not** mix with the other agent's search-replica
files (`maidan-search`, `metrics.rs` replica counters). These are honesty
and small correctness, not features.

| ID | Sev | Scope |
|----|-----|--------|
| **K1** | ✅ done (316) | **A6.** `mail.rs` module docs rewritten to match `notification_router` (wired 249, spawn, presence/digest gates, DLQ 305–306). |
| **K2** | P1 | Strike Open Work "Cluster 180 still open" / `v143` baseline. DM generic-thread hole **shipped** (`ensure_thread_access`). E5 stale-plan pass. |
| **K3** | P1 | Replace `subscribe_resume_secret()` panic with boot-time invariant (assert in `AppState::new` / `main`) or `Option` + 500. Do not leave a panic on a getter. |
| **K4** | P1 | `AUTH_DISABLED` + missing `MAIDAN_SESSION_SECRET` must not silently use `TEST_SUBSCRIBE_RESUME_SECRET` except in tests. Fail boot, or require an explicit `MAIDAN_ALLOW_INSECURE_RESUME_SECRET=1` next to the existing insecure-auth ack. |
| **K5** | P1 | When provider is `hash-v1`, `tracing::warn` at boot: not semantic; set `MAIDAN_EMBEDDING_PROVIDER=openai-compatible` for real search. Pair with E4 README. |
| **K6** | P2 | Delete unused `member_kind_is_human` (or use it). `#[allow(dead_code)]` with zero callers. |
| **K7** | P2 | Rename `openapi/paths/extensions.rs` banner ("Cluster 77 missing") so it does not claim routes are unwired. Keep phantoms; they are utoipa. C3 owns spec=binary. |
| **K8** | P1 | Outbox `list_pending`: add `FOR UPDATE SKIP LOCKED` (steal from `claim_next` / schedules) so two replicas cannot relay the same row. Not Redis. Not batched `pg_notify`. |
| **K9** | P2 | Log (metrics already exist?) failed `advance_delivery_cursor` instead of `let _ =`. At-least-once subscribe depends on that write. |

**Already on other IDs, do not duplicate work:** H5 (Production load sentence), C5/J2 (README MCP freeze), A4 (Production wikilinks), F4 (default-secure demo — include K4).

**Not bugs (do not "fix"):** JSON-RPC notification `let _ = handle`; HMAC `unreachable!`; sqlite-vec `unsafe` init; rate-limit Redis (optional, not the bus).

---

## Suggested order of attack

1. **E2 + E4 + A5 + A6/K1 + K5** — first impression, honesty, `mail.rs` lie, hash-v1 boot warn (half day). **Can run in parallel with 272.**
1b. **K2 + K6 + K7** — Open Work 180 lie, dead `member_kind_is_human`, OpenAPI banner. Docs/dead-code, same half day.
2. **C5 + J3** — honesty copy now; **MCP `2026-07-28` upgrade is P0** (required, not a 2024 freeze). Pack and launch wait on J3. Do not interleave with 272 search files.
3. **A1 + A2** — scrub public types/comments (reputation in `docs.rs` /
   IDE hover). Residue ~771.
4. **E1 + D1 + D2** — examples + evidence (blog ammunition). Hero DAG is Bet 2, not this.
5. **A3 + A4 + E5** — vault/public boundary, wikilinks, freeze stale plans.
6. **D3/K3 + K4 + F1 + F4** — resume-secret panic + test-secret fallback + advisory + secure quickstart.
7. **H1 + H5** — loadgen Postgres baseline + Production.md honesty (measurement, parallel with 270–272).
7b. **I1** — `docs/Providers.md` matrix (docs-only, parallel). Then I2–I5 as compose/recipes, not new dialects.
7c. **J1 + J2** — `docs/Protocols.md` + holding-pattern copy (docs-only). **J3 is P0** once 272 is off the MCP files (or a dedicated MCP branch). J4–J5 after J3.
8. **B1 → B3** — module splits once narrative noise is gone (reviewable diffs). Do not interleave with the other agent's import/search PRs.
9. **H2 → H4** — agent-shaped load mix, then *measured* optimizations only. **K8** (outbox SKIP LOCKED) may ride with H4 if you already have replica evidence; otherwise its own small PR.
9b. **K9** — log failed delivery-cursor advance (tiny, anytime).
10. **C\*** and **D4** — contract consistency + intentional coverage.
11. **G\*** — templates and support norms right before announce.
12. **H3 + H6** — optional nightly soak; reconcile scale-1.0 gate budgets.

Do **not** pick up Clusters 269–272. Do **not** treat 265–266 or
federation egress as unfinished product work.

---

## Definition of done — "ready to show the world"

You may brag when **all** of the following are true:

1. Cold clone: README → one example → healthy server → MCP or REST
   message round-trip **without** reading Clusters/ or Open Work.
2. IDE hover on public types reads like a product, not a changelog.
3. Every claim in the blog post maps to a line in Evidence/Gates + a
   CI job or script.
4. CONTRIBUTING/SECURITY maturity language matches tagged gates.
5. No known request-path `panic!` that should be a typed error; advisory
   ignores have public justifications.
6. mdBook Integrate/Reference pages have working Markdown links (no
   raw `[[wikilinks]]`).
7. You personally re-read Architecture + Integration + Threat Model in
   one sitting and are not embarrassed by drift.
8. `mail.rs` module docs match the router. README does not imply MCP
   2026-07-28 unless the server negotiates it.
9. A dated Postgres loadgen baseline is checked in (H1). Production.md
   no longer claims load/throughput is uncovered.
10. `docs/Providers.md` exists and a cold reader can pick Postgres-host +
    S3-compatible + openai-compatible embeddings + OIDC without reading
    cluster retros.
11. MCP current rev is **`2026-07-28`** (`SUPPORTED_PROTOCOL_VERSIONS` +
    README). A cold reader is not told we speak 2026 while the server is 2024-only,
    and we do **not** launch still speaking only 2024.

Until then: keep shipping, but treat announce/blog as blocked on this
checklist — not on more features. The announce *process* (public-preview
tag, Show HN, claims sheet) is [Launch.md](Launch.md).

---

## How to work this checklist

- Check boxes in PRs that close an item; mention `Pre-Public Hardening`
  in the PR body.
- If an item is declined, strike it with a one-line reason (same
  discipline as Open Work deferrals) so the doc stays honest.
- Re-scan quarterly:

```sh
rg -n 'Cluster [0-9]|PR #[0-9]' crates | wc -l
rg -n 'todo!\(|unimplemented!\(|FIXME' crates
rg -n '\[\[|\]\]' docs --glob '!Clusters/**' --glob '!Retros/**' | head
```

---

## See also

- [Open Work.md](Open%20Work.md) — product/risk backlog (not this doc)
- [Remaining Work.md](Remaining%20Work.md) — Slack parity / exhaustive matrix
- [Expansion Bets.md](Expansion%20Bets.md) — feature expansion after 270–272 (not this doc)
- [Path to Impressive.md](Path%20to%20Impressive.md) — strategy companion
- [Handoff.md](Handoff.md) — session pickup / master ID list
- [Launch.md](Launch.md) — when to announce; extras L1–L6 on top of this DoD
- [Providers.md](Providers.md) — host matrix (Hardening I)
- [Protocols.md](Protocols.md) — wire matrix (Hardening J)
- [Operations.md](Operations.md) — PR/CI/release mechanics
- [Threat-Model.md](Threat-Model.md) — security assets
- `AGENTS.md` — integrator entry (keep thin)
- `CLAUDE.md` — in-repo agent operating manual
