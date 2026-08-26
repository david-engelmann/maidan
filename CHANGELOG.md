# Changelog

All notable changes to Maidan are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [280.0.0] — 2026-08-26

Post-gate hardening (Phase XXIV). **Launch-readiness P1: framework integration
recipes.** No new gate tag.

### Added

- **Framework integration recipes (LangChain / AutoGen / REST).** Copy-paste,
  live-verified clients so an integrator can point an agent framework at Maidan's MCP
  endpoint in minutes. `examples/langchain_maidan.py` loads all 78 tools via
  `MultiServerMCPClient` over Streamable HTTP; `examples/autogen_maidan.py` via
  `StreamableHttpServerParams` + `mcp_server_tools`; `examples/rest_maidan.py` is a
  framework-independent `httpx` client. New `docs/Framework Integrations.md` (in the
  published book) carries the endpoint/token contract, the load-bearing `mcp>=1.9,<2`
  pin (the SDK 2.x stateless rewrite drops modules the current adapters import), and
  AutoGen's every-parameter-needs-a-`type` requirement. Verified against a live Maidan.

### Fixed

- **Every MCP catalog tool parameter now declares a JSON-Schema `type`.**
  `set_thread_result`'s `result` parameter was untyped; AutoGen converts each tool
  schema to a strict Pydantic model and rejected it. Given `"type": "object"`.

## [279.0.0] — 2026-08-26

Post-gate hardening (Phase XXIV). **Launch-readiness P0: production-safe first-admin
bootstrap.** No new gate tag.

### Added

- **`maidan init`.** A one-time CLI bootstrap that seeds the first workspace, an admin
  member, and an all-capabilities bearer token directly through the store (running
  migrations first), prints the token **once**, and **refuses if the database already
  has a workspace**. This removes the "need an admin token to create the first admin
  token" chicken-and-egg: a production deployment needs no `AUTH_DISABLED` and no public
  bootstrap HTTP routes (the image can stay `--no-default-features`). New
  `maidan_auth::capability::all()` exposes the full capability set as the superuser
  grant. Documented in Production.md; guarded by a `maidan-cli` integration test.

## [278.0.0] — 2026-08-26

Post-gate hardening (Phase XXIV). **Launch-readiness P0: one-command quickstart.** No
new gate tag.

### Added

- **One-command quickstart.** `docker compose -f compose.quickstart.yaml up -d --build`
  then `./scripts/quickstart-two-agents.sh` takes a clean machine to two agents
  collaborating in minutes, with no Rust toolchain. `docker/Dockerfile.quickstart`
  pulls a pinned, SHA-256-verified `v277.0.0` release binary (per-arch) onto
  `ubuntu:24.04`, runs non-root, and pre-chowns `/data` so a fresh SQLite/localfs volume
  is writable; `compose.quickstart.yaml` runs one SQLite service on loopback with the
  dev `AUTH_DISABLED` + `MAIDAN_ALLOW_INSECURE_NO_AUTH` acknowledgement; the demo script
  creates a workspace, two agent members, a channel and thread, then posts/reads/replies
  to show durable shared state. Built and run end-to-end locally (`/health` reports the
  real version, no SQLite lock). The compose-smoke CI job validates the quickstart files
  (`compose config` + `bash -n`).

## [277.0.0] — 2026-08-26

Post-gate hardening (Phase XXIV). **Launch-readiness P0: SQLite write-contention
deadlock.** No new gate tag.

### Fixed

- **SQLite "database is locked" on concurrent writes.** SQLite is single-writer and
  sqlx's `pool.begin()` is deferred, so a multi-connection pool lets two writers each
  take a read snapshot and race to upgrade — a genuine deadlock `busy_timeout` cannot
  resolve (a contention harness showed a warm 8-connection pool failing ~90% of
  read-modify-write transactions; one connection is clean). The SQLite backend now
  defaults to **one connection** (`maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS`,
  overridable via `MAIDAN_DB_MAX_CONNECTIONS`); WAL keeps it fast, and Postgres
  (production/HA) is unaffected. Guarded by a new `sqlite_write_contention` test that
  runs the harness at the shipped default and asserts zero lock failures.

## [276.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Launch-readiness P0: runtime version
truthfulness.** No new gate tag.

### Fixed

- **`/health` reported `0.0.0` instead of the release version.** The `version()`
  override (`MAIDAN_VERSION`) already existed, but the release pipeline never set it.
  Now the release tag is baked into every build path: native binaries (`release.yml`
  sets `MAIDAN_VERSION` on the build step), the aarch64 cross build (new `Cross.toml`
  `passthrough`), and the server image (`Dockerfile` `ARG`/`ENV` fed by `build-args`).
  A new `maidan-server/build.rs` declares `rerun-if-env-changed=MAIDAN_VERSION` so a
  warm build cache can't ship a stale version on a source-unchanged (for example
  docs-only) release. Cargo `version = "0.0.0"` intentionally stays (`publish = false`);
  the release identity is the tag, reported at runtime via `MAIDAN_VERSION`. Verified
  locally (the tag string is baked into the binary); the release paths are proven by
  the `v276.0.0` release run.

## [275.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Docs: the final launch pitch and tagline.** No
new gate tag.

### Changed

- **New pitch and tagline.** *"The operating layer for teams of AI agents"* (with the
  supporting line *"Run your agents as one coordinated team that works from a shared,
  durable memory and spends only the tokens it needs"*) now leads the README,
  `docs/Integration.md`, `docs/Architecture.md`, and the OpenAPI `info.description`. The
  body names the gap (gluing together memory + queue + state + pub/sub + auth, while
  agents reload their whole history every turn and miss each other's work) and the
  combination that closes it (coordinate, keep a durable searchable record, pull exactly
  the context a step needs, all under scoped access), for the outcome of better work at
  far fewer tokens. Access control is now first-class in the pitch. Supersedes the
  Cluster-274 "brilliant and forgetful" hook. Removed em-dashes and punchy-fragment
  constructions from the pitch and the README's first screen so it reads as
  human-written.

## [274.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Docs/positioning: new pitch + folded a
public-launch-readiness review into the backlog.** No new gate tag.

### Changed

- **New pitch — off "Slack for agents".** A problem-first hook ("AI agents are
  brilliant and forgetful. Maidan gives a team of them a shared, durable place to
  work.") now leads the README, `docs/Integration.md`, `docs/Architecture.md`, and the
  OpenAPI `info.description`, foregrounding the durable-shared-workspace story
  (transactional state+event, self-healing event log, capability scoping) over the
  chat-app analogy.
- **A2A relabeled as an experimental Maidan subset** (README + Integration) — it is not
  a drop-in A2A v1.0 server. Added a "What Maidan is not" note (not an LLM runtime /
  orchestration planner / hosted SaaS).

### Fixed

- **Broken README quickstart command.** `AUTH_DISABLED=1 cargo run` has failed closed
  since Cluster 157 (needs the explicit `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` ack);
  corrected and explained. Refreshed `docs/Architecture.md`'s stale baseline (`v179` →
  `v273`).

### Added

- **Public-launch-readiness backlog** (`docs/Open Work.md`), folding a verified external
  review of the released binary: runtime-version truthfulness (`/health` reports
  `0.0.0`), a SQLite first-write `database is locked` regression, a one-command
  quickstart, `maidan init`, LangChain/AutoGen recipes + interop CI, published benchmark
  methodology, A2A v1.0 compliance (seeded by the review's gap matrix), and GitHub
  metadata.

## [273.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Docs/governance: reconcile the 2026-08-25
strategy pack into the canonical backlog.** No new gate tag. Docs-only.

### Changed

- **Single source of truth for the backlog restored.** A separate agent's 8-doc
  strategy pack (Handoff, Pre-Public Hardening, Path to Impressive, Expansion Bets,
  Launch, Promotion, Protocols, Providers) had installed a competing backlog (Handoff.md
  said "do not start from Open Work/Roadmap"; CLAUDE.md/README were edited to enforce
  it). A per-doc agent review found the content accurate and tree-grounded, so the pack
  is **kept**
  — but the redirect is reverted: Open Work.md / Roadmap.md remain canonical, and
  Handoff.md is reframed as the *strategy index* that feeds them. The pack's
  genuinely-open items are folded into a new "Post-272 forward work" section in Open
  Work.md (MCP `2026-07-28` stateless upgrade [web-verified as a real spec], durable
  mail retry queue, MCP example pack, client SDKs, Slack/Git projectors, pre-public
  cleanup nits, launch).

### Fixed

- **Docs build.** The pack red the `mdbook` linkcheck gate on a dead link to an
  unpublished retro (`Expansion Bets.md`) and a `- [x]` task-list checkbox parsed as a
  broken reference-link (`Providers.md`); both fixed — a local `mdbook build`
  (linkcheck, `warning-policy = error`) passes with zero errors. Also corrected
  same-day staleness (the pack was drafted while 270–272 were in flight): refreshed the
  "v269 / in-flight" snapshots, Open Work's "latest v251" + "269–272 remaining", and
  CLAUDE.md's "latest v268" orientation pointer, to reflect 267–272 shipped (tags
  through `v273`). Reframed the unregistered `maidan.world` domain (verified `NXDOMAIN`)
  as *planned* rather than the live "Published/canonical" site in AGENTS.md/README.md/
  Promotion.md — GitHub Pages stays the live site today.

## [272.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Optional deferrals sweep, part 6 (final) — search
read-routing observability.** No new gate tag.

### Added

- **`maidan_search_replica_reads_total{outcome="primary"|"replica"}`** — the
  search-side twin of `maidan_replica_reads_total`, so the primary-vs-replica split for
  message search is visible independently of store reads. `PostgresSearch` gets a
  metrics-agnostic `SearchReadMetrics` (two atomics) incremented in `read_pool()` only
  when a replica is configured; `main.rs` captures the handle (when
  `MAIDAN_DB_REPLICA_URL` is set) onto `AppState`, and `metrics.rs` delta-syncs it into
  the counter each tick. No separate lag gauge — the store's poller already emits
  `maidan_replica_lag_bytes` for the same replica. Counter assertions added to the
  `#[ignore]`d real-replica `replica_routing` test. **Closes the optional-deferrals
  sweep (267–272) and the LSN read-replica program end-to-end.**

## [271.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Optional deferrals sweep, part 5 — search
token-aware read routing.** No new gate tag.

### Added

- **Message search honors the read-consistency token.** `maidan-search`'s
  `PostgresSearch` now routes its reads to a Postgres read replica once it has caught
  up to the request's `Maidan-Consistency-Token` — the search-side twin of the
  Clusters 262–266 store routing. It gains its own `reader` pool + a 200 ms replica
  replay-LSN poller + a `read_pool()` selector; `new(pool)` is byte-unchanged for
  single-primary deployments, `with_replica_reader` wires a replica. The decision is
  single-sourced via a new `maidan_store::postgres::replica_route(has_replica, cached)`
  that reads the same `READ_CONSISTENCY` task-local the store uses. Lexical + semantic
  reads route (semantic resolves its model table and runs its query against the same
  pool so they never disagree); embedding writes / index DDL / reindex stay on the
  primary. `main.rs` builds search its own replica reader when `MAIDAN_DB_REPLICA_URL`
  is set. Validated against real streaming replication (`#[ignore]`d `replica_routing`
  test + `scripts/replica-harness.sh`).

## [270.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Optional deferrals sweep, part 4 — workspace
import, the route.** No new gate tag.

### Added

- **`POST /workspaces/import`** (`token:admin`) — the write-side inverse of the
  Cluster-187 export, over the Cluster-269 `Store::import_workspace`. The body is
  exactly the `GET /workspaces/{id}/export` bundle (`WorkspaceExport` gained
  `Deserialize`), so export → import round-trips. Two modes (`?mode=`): **new**
  (default) remaps every id to a fresh one and lands a brand-new workspace (never
  collides); **restore** preserves the bundle's ids — **409** if that workspace
  already exists, unless `&force=true` erases it first and restores over it (disaster
  recovery). The pure `import::remap` (fresh ids + full FK rewrite) and `import::flatten`
  are unit-tested for referential integrity; the route is proven end-to-end
  (`workspace_import_e2e`). Reactions/votes and artifact blobs remain outside the
  bundle (Cluster-187 scope), so a round-trip drops them — documented.

## [269.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Optional deferrals sweep, part 3 — workspace
import, store foundation.** No new gate tag.

### Added

- **Workspace import (store).** `WorkspaceImport` (the deserializable mirror of the
  Cluster-187 `WorkspaceExport`) + `Store::import_workspace` — one transaction,
  all-or-nothing, full-column inserts that preserve explicit ids, state, and
  timestamps, so an exported bundle round-trips faithfully. Both backends
  (`postgres/import.rs` binds JSONB `metadata`/`content` directly; `sqlite/import.rs`
  stores them as JSON TEXT); `message_edits.id` (an unreferenced serial) regenerates,
  every other id is explicit. Zero-blast-radius store foundation — no routes, no
  remap, no conflict guard yet (those are Cluster 270). Both-backend round-trip test
  covers a private channel, a closed+assigned thread, structured content, a
  tombstoned message, an edit, a pin, and a reference.

## [268.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Optional deferrals sweep, part 2.** No new gate tag.

### Added

- **MCP email-address tools.** `set_member_email` / `get_member_email` /
  `delete_member_email` (`workspace:read`, member-scoped) — the MCP twins of the
  Cluster-250 REST over the Cluster-248 store, so an MCP-only agent can manage a
  member's delivery address. `set` does a light `@` check (→ `InvalidParams`), `get`
  returns the address or `null`, `delete` returns `{deleted}`. No new store logic.

## [267.0.0] — 2026-08-25

Post-gate hardening (Phase XXIV). **Optional deferrals sweep, part 1.** No new gate tag.

### Added

- **A2A egress: content → parts.** `message_parts_from_content` (the inverse of
  Cluster-194's `message_content`), and the A2A `SendMessage` agent now renders its
  outbound message from the stored message's canonical structured `content` (each
  block projected to text, mirroring `derive_body`) instead of echoing the request —
  so an A2A consumer sees Maidan's stored representation. Behaviour-preserving for
  A2A-ingested (text) messages, which round-trip faithfully. Closes the federation
  egress deferral (event relay already carried `content`).

## [266.0.0] — 2026-08-24

Post-gate hardening (Phase XXIV). **Program D (scale & durability) — read-replica
arc, part 6 (closer).** No new gate tag.

### Added

- **Replica-lag gauge + read-replica docs.** `maidan_replica_lag_bytes` (the LSN
  poller now also samples the primary's `pg_current_wal_lsn()` and reports
  `current − replay` in WAL bytes), and a "Read replicas" section in
  `docs/Production.md` documenting `MAIDAN_DB_REPLICA_URL`, the `Maidan-Consistency-Token`
  read-your-writes contract, what routes to the replica (content GETs) vs what
  always hits the primary (writes, auth reads, control-plane reads), the metrics,
  and the local test harness. **Completes the LSN causality-token read-replica arc
  (Clusters 261–266) and Program D (scale & durability).**

## [265.0.0] — 2026-08-24

Post-gate hardening (Phase XXIV). **Program D (scale & durability) — read-replica
arc, part 5.** No new gate tag.

### Added

- **Remaining read families routed + a routing metric.** The rest of the
  content/collaboration reads (skills, thread results, notifications, follows,
  emails, last-seen, channel members, DMs/group DMs, thread transitions, queue
  depth, task schedules, assignments, dependencies, message edits, mentions, inbox,
  votes, reactions, workspace usage — 28 delegations) now route to the read replica
  under a request's consistency token, completing the member-facing read surface
  begun in Cluster 264. Auth-path reads (sessions, API tokens, OIDC, federation
  peers) and control-plane/config reads (webhooks, slash commands, fsm hooks,
  automation deliveries, reindex jobs, audit, token quotas) deliberately **stay on
  the primary** — the auth middleware runs on GETs, so a just-minted credential must
  not be read from a lagging replica. New `maidan_replica_reads_total{outcome=primary|replica}`
  metric (a store-side `ReadRoutingMetrics` counter surfaced by the server). Inert
  without a replica.

## [264.0.0] — 2026-08-24

Post-gate hardening (Phase XXIV). **Program D (scale & durability) — read-replica
arc, part 4.** No new gate tag.

### Added

- **Token-aware read routing.** GET/HEAD requests now run inside a read-consistency
  scope (a `tokio` task-local carrying the parsed `Maidan-Consistency-Token`), and
  `PostgresStore` routes the core entity reads (workspace/member/channel/thread/
  message get + list) to the replica **only once it has replayed past the token** —
  otherwise the primary (read-your-writes). A background poller caches the replica's
  `pg_last_wal_replay_lsn()` in an atomic, so the per-read decision is a cheap
  compare with no extra round-trip (a stale cache is safe — it can only route to the
  primary unnecessarily, never serve a stale read). Mutation handlers and background
  workers are not scoped, so their reads always hit the primary (no read-then-write
  staleness). Validated end-to-end against real streaming replication; inert without
  a replica. Remaining read families + metrics come in Cluster 265.

## [263.0.0] — 2026-08-22

Post-gate hardening (Phase XXIV). **Program D (scale & durability) — read-replica
arc, part 3.** No new gate tag.

### Added

- **Consistency token on writes.** After a successful mutation, when a read replica
  is configured (`MAIDAN_DB_REPLICA_URL`), the server stamps the primary's WAL LSN on
  the response as `Maidan-Consistency-Token` — the causality token a client echoes on
  a later read so replica reads never go staler than the client's own writes.
  Backed by a new `Store::write_lsn()` (Postgres `pg_current_wal_lsn()`; SQLite
  `None`), an `AppState.read_replica_enabled` flag, and a response middleware. The
  LSN is captured after the handler (safely over-approximating — never behind the
  write). No replica configured → no token and no extra round-trip (unchanged).
  Cluster 264 will ingest the token and route reads.

## [262.0.0] — 2026-08-22

Post-gate hardening (Phase XXIV). **Program D (scale & durability) — read-replica
arc, part 2.** No new gate tag.

### Added

- **Reader-pool split (inert).** `PostgresStore` now holds a distinct `reader` pool
  (`with_replica_reader` constructor; `new` defaults `reader` to a clone of the
  writer pool, so the ~62 existing call sites and default behaviour are unchanged),
  and the server connects a real read replica at boot when `MAIDAN_DB_REPLICA_URL`
  is set (validating reachability, with the same connection setup as the primary).
  Reads still go to the primary — the token-aware `read_pool` selector arrives in a
  later cluster. Unset `MAIDAN_DB_REPLICA_URL` → zero behaviour change.

## [261.0.0] — 2026-08-22

Post-gate hardening (Phase XXIV). **Program D (scale & durability), part 4 —
read-replica arc, part 1.** No new gate tag.

### Added

- **LSN causality-token primitives + a real streaming-replication harness.** The
  foundation of LSN-token read-replica routing: an `Lsn` type (`maidan-types`,
  `u64`-backed so `pg_lsn` values order numerically — `0/9 < 0/10` — with
  `from_pg_str`/`to_pg_str`), store helpers `current_wal_lsn` / `replica_replay_lsn`
  / `replica_caught_up` (`maidan-store::postgres::replication`, called directly like
  the bus's `get_by_id` — no `Store`-trait/SQLite ripple), and
  `scripts/replica-harness.sh` that stands up a local pgvector primary + streaming
  standby. An `#[ignore]`d test (`maidan-store/tests/replication.rs`, connects via
  `MAIDAN_PRIMARY_URL`/`MAIDAN_REPLICA_URL`) validates the helpers against real
  replication — the standby catches up to the primary's write LSN and replicated
  rows are visible. The `Lsn` unit tests run in CI; the replication test is a manual
  tool (needs Docker). Inert — nothing routes reads yet.

## [260.0.0] — 2026-08-21

Post-gate hardening (Phase XXIV). **Program D (scale & durability), part 3.** No new
gate tag.

### Added

- **Backup / restore + disaster-recovery runbook.** `scripts/backup.sh` (`pg_dump
  -Fc` of `DATABASE_URL` + a `tar` of the `localfs` artifact root + a manifest) and
  `scripts/restore.sh` (`pg_restore` into the target + untar; refuses a non-empty
  target unless `--force`, then restores with `--clean --if-exists`), plus a
  "Backup & disaster recovery" section in `docs/Production.md` documenting coverage,
  the secrets that must be restored out of band (`MAIDAN_SESSION_SECRET`, the
  `FEDERATION_ENCRYPTION_KEY` keyring, SMTP/OIDC creds), S3 as its own durable store,
  RPO/RTO guidance (periodic backup vs WAL/PITR), and the step-by-step recovery
  sequence. Operator tools like `loadgen`/`chaos` — `bash -n`-clean, not CI-gated.

## [259.0.0] — 2026-08-21

Post-gate hardening (Phase XXIV). **Program D (scale & durability), part 2.** No new
gate tag.

### Added

- **Chaos / fault-injection harness.** `crates/maidan-bus/tests/chaos.rs` +
  `scripts/chaos.sh`: an `#[ignore]`d soak that publishes at a `PostgresBus` under
  load while repeatedly killing the `LISTEN` backend connection
  (`pg_terminate_backend` on connections running a `LISTEN`), then asserts every
  published event still reached the local broadcast — validating that the
  Cluster-258 self-healing NOTIFY floor back-fills what the dropped notifications
  would have delivered. Measured locally: 40 published, 40 delivered, 5 listener
  kills, 0 missing. Like the Cluster-198 load harness, the soak is `#[ignore]`d (it
  needs Docker and is timing-sensitive — a resilience tool, not a CI gate); the pure
  `fault_due` schedule helper is unit-tested in CI. Env knobs `MAIDAN_CHAOS_OPS` /
  `MAIDAN_CHAOS_KILL_EVERY` / `MAIDAN_CHAOS_DELAY_MS`.

## [258.0.0] — 2026-08-21

Post-gate hardening (Phase XXIV). **Program D (scale & durability), part 1.** No new
gate tag.

### Added

- **Event-bus self-healing NOTIFY floor.** The Postgres `LISTEN`/`NOTIFY` bus now
  tracks a high-water `log_id` and back-fills the missed range from the event log on
  a detected gap (a pointer id above `high_water + 1`) or on a listener reconnect
  (drain to head after an error) — so events appended while the `LISTEN` was
  disconnected still reach the local broadcast instead of being silently dropped on
  the optimistic path. The pointer's own id is always hydrated (never skipped on
  `<= high_water`, so a concurrently-committed lower id is never lost); back-fill is
  batched and best-effort. New cross-workspace log reads `list_after_global` /
  `max_event_id`, a `Backfilled` hydrate stat +
  `maidan_bus_notify_hydrate_total{result="backfilled"}`, and a `PostgresBus::backfill`
  heal hook. This is an optimistic-path resilience improvement; the transactional
  outbox + at-least-once cursor remain the durable delivery path.

## [257.0.0] — 2026-08-21

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 21** —
Arc I. No new gate tag.

### Added

- **Delivery-mode MCP tools.** `set_delivery_mode` and `get_delivery_mode`
  (`workspace:read`, member-scoped) — the MCP twins of the Cluster-256 REST, so an
  MCP-only agent can read and switch a member between immediate per-notification
  emails and a periodic digest. `set_delivery_mode` parses a snake_case `mode`
  (`immediate` / `digest`) and returns `InvalidParams` on an unknown one; both return
  `{mode}`. Standard 5-place wiring + both sorted contract JSONs. Closes the core of
  Arc I — the digest feature is now reachable over REST and MCP.

## [256.0.0] — 2026-08-20

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 20** —
Arc I. No new gate tag.

### Added

- **Delivery-mode REST.** `PUT /members/:id/delivery-mode` (set) and
  `GET /members/:id/delivery-mode` (read; `immediate` when never set), both
  `workspace:read` and self-only for a session caller. A member can now choose
  between immediate per-notification emails and a periodic digest over the API
  (previously store-only). The request DTO wraps `EmailDeliveryMode` directly, so an
  unknown mode is a `400` at the extractor. Full new-route preflight (OpenAPI +
  capability-map + matrix).

## [255.0.0] — 2026-08-20

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 19** —
Arc I. No new gate tag.

### Added

- **Digest sweeper + router honors digest mode.** The notification router now skips
  the immediate per-notification email for a member in `Digest` delivery mode
  (Cluster 254), metered `maidan_email_delivered_total{outcome="skipped_digest"}`;
  and a new opt-in background digest sweeper (`MAIDAN_DIGEST_TICK_SECS`) drains
  `members_due_for_digest`, emails each digest-mode member an unread-count rollup,
  and advances their digest watermark **only on a successful send** (so a transient
  failure retries next tick, at-least-once). The alternative-mode digest now works
  end-to-end. The sweeper is a no-op without a mail transport and — deliberately,
  unlike the scheduler's `SKIP LOCKED` claim — is not single-flighted across
  replicas (a duplicate digest is low-harm; run it on one replica for exactly-once).

## [254.0.0] — 2026-08-20

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 18** —
Arc I. No new gate tag.

### Added

- **Email digest data model (store foundation).** The store layer for scheduled
  email digests: an `EmailDeliveryMode` (`Immediate` default / `Digest`) and a
  `DigestDue` enumeration row in maidan-types; two per-member tables
  (`maidan_member_delivery_prefs`, `maidan_member_digest_state`; pg 0048 / sqlite
  0047); and store `set_delivery_mode` / `get_delivery_mode` (default `Immediate`
  when unset) / `set_last_digest_at` (the digest watermark) / `members_due_for_digest`
  (digest-mode members with an address and unread notifications created since their
  last digest, address carried inline), both backends. Implements the chosen
  alternative-mode product — a member picks immediate per-notification emails *or* a
  periodic digest, not both. **Foundation** — no router change, sweeper, or routes
  yet, so zero behaviour change until Cluster 255.

## [253.0.0] — 2026-08-20

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 17** —
Arc I. No new gate tag.

### Added

- **Presence-aware email routing.** The WS `/ws/subscribe` handler now records a
  member's last-seen (`touch_member_last_seen`) on presence registration —
  best-effort and spawned, so it never blocks the connect — and
  `deliver_notification_email` consults it: with `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS`
  set to a positive value, a notification email is suppressed when the recipient was
  seen within the window (they are active and will see the in-app notification),
  metered as `maidan_email_delivered_total{outcome="skipped_present"}`. **Opt-in:**
  unset or `0` disables the guard and every opted-in recipient is emailed, exactly
  the Cluster-249 behaviour — a positive window is required to enable suppression.
  A lookup error falls through and sends (a transient read never drops an email).
  Wires the Cluster-252 durable last-seen store end-to-end.

## [252.0.0] — 2026-08-20

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 16** —
Arc I. No new gate tag.

### Added

- **Durable member last-seen (store foundation).** A `maidan_member_last_seen`
  table (pg 0047 / sqlite 0046; `member_id` PK, `last_seen_at`) + store
  `touch_member_last_seen` (idempotent upsert to `now()`) and `get_member_last_seen`
  (→ `Option<DateTime<Utc>>`), both backends. Presence is in-memory only today, so it
  can't say "was this member recently active?" after a restart or across replicas;
  this gives presence-aware email routing (Cluster 253) a persistent signal. A
  separate one-row-per-member table (not a column on `maidan_members`) avoids the
  member-row schema ripple; no `MemberLastSeen` model (the row is just a timestamp).
  **Foundation** — nothing calls `touch` or reads `get` yet, so zero behaviour
  change until 253.

## [251.0.0] — 2026-08-20

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 15** —
Arc I. No new gate tag.

### Added

- **`/ui` notification center.** A "Notifications" tab in the `/ui` lists the
  signed-in member's notifications (with an unread-count badge, an unread-only
  filter, per-item "Mark read", and "Mark all read"), over four new
  `/ui/api/members/:id/notifications*` routes that reuse the Cluster-239 handlers
  under the session middleware. `sessionMemberId` is passed as `:id`, so the
  self-only guard lets a session user see only their own inbox. No new handlers,
  capability-map, or OpenAPI churn (`/ui/api` is a curated subset absent from
  OpenAPI); the JS uses only established helpers so the `ui_js_contract` guard stays
  green. The whole notification system now has a human face.

## [250.0.0] — 2026-08-20

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 14** —
Arc I. No new gate tag.

### Added

- **Member delivery-email REST.** `PUT /members/:id/email` (set — opt in),
  `GET /members/:id/email` (read; `404` when unset), and `DELETE /members/:id/email`
  (clear — opt out), all `workspace:read` and self-only for a session caller. With the
  Cluster-249 router wiring, the email feature now works end-to-end over REST: a
  member registers an address and their notifications arrive by email (when SMTP is
  configured). `PUT` does a light `@` sanity check (`400` on obvious garbage); full
  validation stays at the transport. MCP email tools are an optional low-value
  follow-up.

## [249.0.0] — 2026-08-20

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 13** —
Arc I. No new gate tag.

### Added

- **Email delivery wired into the notification router.** When the router writes a
  per-recipient notification, it now also delivers it by email to members who have a
  delivery address (Cluster 248) — if an SMTP transport is configured (Cluster 247).
  `AppState.mail` is built from `SmtpConfig::from_env` in `main.rs` (`None`, so no
  email, unless `MAIDAN_SMTP_*` is set and in tests); the send is spawned best-effort
  after the in-app notification write so a slow/failing SMTP server never blocks
  routing (a failure is logged + counted via `maidan_email_delivered_total{outcome}`,
  not retried). Presence of an address is the opt-in. The REST/MCP surface to set an
  address (Cluster 250) and a durable retrying delivery queue are follow-ups.

## [248.0.0] — 2026-08-19

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 12** —
Arc I. No new gate tag.

### Added

- **Member delivery-email store.** A `maidan_member_emails` table (pg 0046 / sqlite
  0045; `member_id` PK, `email`, one per member) + `MemberEmail` model + store
  `set_member_email` (upsert) / `get_member_email` / `delete_member_email`, both
  backends. Where a member's email notifications go — the recipient-address
  prerequisite for the Cluster-247 SMTP transport. A separate table (not a column on
  `maidan_members`) to avoid the shared-member-row ripple. **No delivery wiring yet**
  — the router/worker + delivery preference + REST/MCP follow in Cluster 249.

## [247.0.0] — 2026-08-19

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 11** —
opens **Arc I (transport + reach)**. No new gate tag.

### Added

- **Email/SMTP transport foundation.** A `MailTransport` trait + a `lettre`-backed
  `SmtpTransport` + `SmtpConfig::from_env` (`MAIDAN_SMTP_HOST` / `PORT` / `USERNAME` /
  `PASSWORD` / `FROM` / `STARTTLS`), the first off-platform delivery transport (beyond
  webhook `deliver_http`). **Config-gated** — no `MAIDAN_SMTP_*` means no mailer is
  built and nothing is sent — and **not wired into the router yet**. `lettre` uses the
  existing rustls + tokio stack (no openssl); `cargo deny` passes with `0BSD` added to
  the licence allow-list (lettre is BSD-Zero-Clause). Recipient email addresses
  (Cluster 248) and delivery wiring (249) follow.

## [246.0.0] — 2026-08-19

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 10** —
**closes Arc H**. No new gate tag.

### Added

- **Follows MCP tools.** `follow_channel` / `unfollow_channel` / `list_channel_follows`
  + the thread triple — the MCP twins of Cluster 245's REST, over the shared store
  (`workspace:read`, member-scoped). `follow_channel` / `follow_thread` gate on access
  to the target (they join the pre-dispatch channel/thread access gates); unfollow and
  list don't. **This completes Arc H** — preferences + subscription (mute 241–243,
  follows 244–246) over REST + MCP, the routing brain the notification router consults.

## [245.0.0] — 2026-08-19

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 9** —
Arc H. No new gate tag.

### Added

- **Follows-aware router + follow REST.** The notification router now fans a
  `MessagePosted` to the followers of the message's channel + thread (minus the
  author, and honoring each recipient's mutes) — following a channel or thread now
  delivers new activity to the follower's inbox. `POST`/`GET /members/:id/channel-follows`
  + `DELETE …/:cid` (and the thread triple) let a member follow/unfollow/list, all
  `workspace:read` and self-only for a session caller; following requires access to
  the target (`ensure_channel_access` / `ensure_thread_access`). Note: a member
  mentioned in a channel they also follow gets both a mention and a follow
  notification (distinct events); per-kind mute (`message_posted`) is the control. The
  MCP follow tools follow in 246, closing Arc H.

## [244.0.0] — 2026-08-19

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 8** —
Arc H. No new gate tag.

### Added

- **Follows/subscription foundation.** `maidan_channel_follows` +
  `maidan_thread_follows` tables (pg 0045 / sqlite 0044; PK `(member, target)`, reverse
  index on the target — presence of a row = following) + `ChannelFollow` /
  `ThreadFollow` models + store `follow_channel` / `unfollow_channel` /
  `list_channel_follows` / `channel_followers` and the thread quartet, both backends. A
  member follows a channel or thread to be notified of activity there even without a
  mention; `*_followers` is the router's fan-out set. **No router change or routes
  yet** — the zero-blast-radius foundation pattern (Cluster 230). Opens the
  follows half of Arc H (the mute half completed at 243).

## [243.0.0] — 2026-08-19

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 7** —
Arc H. No new gate tag.

### Added

- **Mute-preference MCP tools.** `set_notification_pref` (upsert a per-`EventKind`
  mute; `kind` is a snake_case string parsed to `EventKind`) and
  `list_notification_prefs` — the MCP twins of Cluster 242's REST, over the shared
  store (`workspace:read`, member-scoped). The **mute** half of Arc H is now complete
  over REST + MCP; the follows/subscription half is next.

## [242.0.0] — 2026-08-19

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 6** —
Arc H. No new gate tag.

### Added

- **Mute-aware router + preferences REST.** The notification router now consults a
  member's preferences: `route_event` skips writing a notification when the recipient
  has muted that `EventKind` (metered as
  `maidan_notifications_suppressed_total{reason=muted}`). `PUT /members/:id/notification-prefs`
  (upsert) and `GET /members/:id/notification-prefs` (list) let a member set/read their
  mutes — `workspace:read` and self-only for a session caller (a member configures
  their own prefs; a bearer is the act-as-any orchestrator, the Cluster-239 model).
  Wires the Cluster-241 foundation into the router; the MCP tools follow in 243.

## [241.0.0] — 2026-08-19

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 5** —
opens **Arc H (preferences + subscription)**. No new gate tag.

### Added

- **Notification mute-preferences foundation.** A `maidan_notification_prefs` table
  (pg 0044 / sqlite 0043; PK `(member_id, kind)`, a `muted` flag — one row per member
  × `EventKind`, absent = notify) + `NotificationPref` model + store
  `set_notification_pref` (upsert) / `list_notification_prefs` /
  `is_notification_muted` (the router's "should I suppress this?" query, defaulting to
  not-muted), both backends. The routing brain the notification router will consult
  before writing. **No router change or routes yet** — the zero-blast-radius
  foundation pattern (Clusters 159 / 230). Opens Arc H after Arc G (per-recipient
  notifications) completed at Cluster 240.

## [240.0.0] — 2026-08-18

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 4** —
**closes Arc G**. No new gate tag.

### Added

- **MCP notification tools + `wait_for_notification`.** `list_notifications` /
  `get_unread_count` / `mark_notification_read` (the MCP twins of Cluster 239's REST,
  over the shared store) + **`wait_for_notification`** — a long-poll that blocks until
  the member's next notification-worthy event (today: mentions), the general form of
  `wait_for_mention` (both now delegate to a shared `wait_for_member_event` helper). An
  MCP-native agent can drain its inbox, clear it, and await new notifications. This
  **completes Arc G** — the per-recipient notification arc (ledger 237 → router 238 →
  REST 239 → MCP 240).

## [239.0.0] — 2026-08-18

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 3** —
Arc G. No new gate tag.

### Added

- **REST unified inbox.** `GET /members/:id/notifications` (list; `unread_only`,
  `limit`), `GET /members/:id/notifications/unread-count` (the badge), `POST
  /members/:id/notifications/:nid/read` (mark one, returns the new count), and `POST
  /members/:id/notifications/read-all` (returns `{cleared}`) — all `workspace:read`
  and **self-only for a session caller** (a member reads their own inbox; a bearer is
  the act-as-any orchestrator, the Cluster-202/203 model). `mark_notification_read` is
  now recipient-scoped in the store (`(member_id, id)`), so a mark can't touch another
  member's notification and a foreign/unknown id returns `404`. The read side of the
  Cluster-237 ledger + Cluster-238 router; the MCP tools + `wait_for_notification`
  (240) close Arc G.

## [238.0.0] — 2026-08-18

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 2** —
Arc G. No new gate tag.

### Added

- **Notification router.** A `NotificationRouter` background worker (an always-on,
  reconnecting event-bus consumer spawned in `main.rs`) resolves each relevant event
  to the members it concerns and writes a per-recipient `maidan_notifications` row.
  Currently routes `MentionRecorded` → a notification for the mentioned member (with
  the channel resolved from the thread). Writes go through a new
  `create_notification_if_absent` (`ON CONFLICT DO NOTHING`) against a new
  `UNIQUE(member_id, source_log_id)` index (pg 0043 / sqlite 0042), so a replayed
  event or a second replica running the consumer cannot double-notify. A new
  `maidan_notifications_created_total{kind}` metric counts real writes. An @mention
  is now *delivered* to the recipient's ledger the moment it hits the bus, not just
  recorded and polled — the unified inbox (239) and `wait_for_notification` (240)
  build on it.

## [237.0.0] — 2026-08-18

Post-gate hardening (Phase XXIV). **Program C (notifications & reach), part 1** —
opens **Arc G (per-recipient notification ledger + router + unified inbox)**. No new
gate tag.

### Added

- **Per-recipient notification ledger.** A `maidan_notifications` table (pg 0042 /
  sqlite 0041; one row per recipient × source event — `member_id` recipient, `kind`
  = the triggering `EventKind`, `source_log_id` = the event-log row (no FK, so it
  survives retention pruning), denormalized `channel/thread/message/actor` for
  rendering, `read_at` NULL = unread) + `Notification` / `NewNotification` model +
  store CRUD (`create_notification`, `list_notifications` newest-first / unread-only,
  `mark_notification_read` idempotent, `mark_all_notifications_read`,
  `unread_notification_count`), both backends. Where a mention was one shared row read
  through a single inbox cursor, this is the per-recipient delivery/read layer the
  notification router + unified inbox build on. **No router or routes yet** — the
  zero-blast-radius foundation pattern (Clusters 159 / 217 / 226 / 230 / 234). Opens
  Program C after Program B (agentic orchestration) completed at Cluster 236.

## [236.0.0] — 2026-08-18

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 20 —
**closes Arc F and Program B**. No new gate tag.

### Added

- **Structured-result MCP surface + coordination wait.** MCP `set_thread_result`
  (`thread:transition`) / `get_thread_result` (`workspace:read`) — the twins of the
  Cluster-235 REST endpoints, over the shared store; `set` publishes the
  `ThreadResultSet` event. **`wait_for_result`** (`workspace:read`) blocks until a
  thread's result is produced (a `ThreadResultSet` for that thread) and returns the
  result payload, or `null` on timeout — the coordination wait for the "spawn
  sub-tasks, wait, aggregate" pattern (the `wait_for_ready` analogue; live-only, read
  `get_thread_result` first for an already-produced result). **`get_dependency_results`**
  (`workspace:read`) lets a parent task gather its dependencies' outputs as
  `[{thread_id, result}]` (result `null` for a dependency that hasn't produced one),
  skipping dependencies in channels the caller can't access. **This closes Program B**
  (agentic orchestration: task-DAG + queue 217–225, scheduled tasks 226–229,
  capability registry + skill routing 230–233, coordination waits + structured
  results 234–236).

## [235.0.0] — 2026-08-18

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 19 — Arc F
(coordination waits + structured results). No new gate tag.

### Added

- **Thread-result REST + `ThreadResultSet` event.** `PUT /threads/:id/result`
  (`thread:transition`) attaches a task's structured JSON result (upsert — a re-set
  overwrites) and `GET /threads/:id/result` (`workspace:read`) reads it back (`404`
  until produced); both enforce the DM-participant-aware thread RBAC. Setting a
  result now publishes a `ThreadResultSet` **event** — a small "go fetch" pointer
  (`{workspace, channel, thread, produced_by}`, no payload inline; a waiter fetches
  via `GET …/result`), observable on the WS + MCP-SSE event streams exactly like
  `ThreadReady` (Cluster 222). Locally-derived, so **non-federatable** (allowlist
  excludes it alongside `ArtifactUpserted` + `ThreadReady`). Wires the store
  foundation from Cluster 234; the MCP surface + a `wait_for_result` long-poll follow
  in 236.

## [234.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 18 — opens
**Arc F (coordination waits + structured results)**. No new gate tag.

### Added

- **Thread-result store foundation.** A `maidan_thread_results` table (pg 0041 /
  sqlite 0040; `thread_id` PK, `result` JSONB/TEXT, `produced_by`, `produced_at`) +
  `ThreadResult` model + `Store::set_thread_result` (upsert — one result per thread,
  a re-set overwrites) / `Store::get_thread_result` (`None` until produced), both
  backends. An agent attaches a structured result when it finishes a task; a
  requester (or a parent task that depends on it) reads it back. **No worker or
  routes yet** — the zero-blast-radius foundation pattern (Clusters 159 / 217 / 226 /
  230). Coordination waits (a `ThreadResultSet` event + `wait_for_result`) follow.

## [233.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 17 —
completes **Arc E (capability registry + skill routing)**. No new gate tag.

### Added

- **Capability-registry MCP tools.** `add_member_skill` / `list_member_skills`
  (member skills; `workspace:write` / `workspace:read`) and
  `add_thread_required_skill` / `list_thread_required_skills` (task requirements;
  `thread:transition` + channel access / `workspace:read`) — the MCP twin of the
  Cluster 232 REST endpoints, over the shared store, so an MCP-only agent can
  declare its skills and set a task's requirements. Full 5-place wiring. **Arc E is
  complete**: skill routing is surfaced over REST + MCP and enforced in `claim_next`
  (Cluster 231).

## [232.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 16 — Arc E
REST surfaces. No new gate tag.

### Added

- **Capability-registry REST.** Declare / list / remove a member's skills
  (`POST`/`GET /members/:id/skills`, `DELETE /members/:id/skills/:skill`;
  `workspace:write` for writes, `workspace:read` for the list) and set / list /
  remove a task's required skills (`POST`/`GET /threads/:id/required-skills`,
  `DELETE …/:skill`; `thread:transition` for writes + thread access,
  `workspace:read` for the list). So an operator (or bearer orchestrator) can drive
  the skill routing from Cluster 231 without touching the store. Full new-route
  preflight (6 routes).

## [231.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 15 — Arc E
skill-aware claim. No new gate tag.

### Added

- **Skill routing — `claim_next` matches a task's required skills.** A
  `maidan_thread_required_skills` table (pg 0040 / sqlite 0039) + `ThreadRequiredSkill`
  model + store CRUD (`add`/`remove`/`list`, both backends), **and** `claim_next` /
  `claim_next_with_event` now skip a task whose required skills the claimer doesn't
  hold — one `NOT EXISTS (required skill NOT IN member's skills)` clause beside the
  Cluster-218 readiness clause (4 SQL sites, both backends). A task with no required
  skills is claimable by anyone (set containment). The existing claim route
  (`POST /channels/:cid/threads/claim-next`) and the `claim_next_thread` MCP tool
  become skill-routing for free. No new claim API.

## [230.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 14 — opens
**Arc E (capability registry + skill routing)**. No new gate tag.

### Added

- **Member-skills store foundation.** A `maidan_member_skills` table (pg 0039 /
  sqlite 0038) + `MemberSkill` model + three `Store` methods (both backends):
  `add_member_skill` (idempotent, rejects an empty skill), `remove_member_skill`
  (conditional), `list_member_skills` (ordered by skill). Skills are free-form tags
  an agent declares it can do; skill routing (later clusters) matches a task's
  required skills against these (set containment). **No worker or routes yet** — the
  zero-blast-radius foundation pattern (Clusters 159 / 217 / 226).

## [229.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 13 — the
scheduler MCP tools; the scheduler subsystem is now surfaced over REST + MCP. No new
gate tag.

### Added

- **Task-schedule MCP tools.** `create_task_schedule` (`workspace:write`,
  channel-gated: `{channel_id, title, interval_secs?, first_run_at?}`) and
  `list_task_schedules` (`workspace:read`; the caller's workspace, filtered to
  channels the caller can access) — so an MCP-only agent can schedule its own
  recurring/one-shot work and inspect what's scheduled. The MCP twin of the Cluster
  228 REST endpoints, over the shared store; full 5-place wiring. The schedule is
  owned by the caller (`created_by = auth.member_id`).

## [228.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 12 — the
scheduler management API. No new gate tag.

### Added

- **Task-schedule REST management API.** `POST /workspaces/:wid/task-schedules`
  (`workspace:write` + target-channel access) creates a schedule
  (`{channel_id, title, interval_secs?, first_run_at?}` — `interval_secs` omitted =
  one-shot; `first_run_at` omitted = next tick); `GET /workspaces/:wid/task-schedules`
  (`workspace:read`) lists; `PUT /task-schedules/:id` (`workspace:write`) pauses /
  resumes via `{active}`; `DELETE /task-schedules/:id` (`workspace:write`) removes.
  Management surfaces resolve the schedule and enforce workspace + target-channel
  access. New `Store::set_task_schedule_active`. So an operator can drive the
  scheduler (Cluster 227) without touching the store directly.

## [227.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 11 — the
scheduler worker. No new gate tag.

### Added

- **Scheduler sweeper worker.** A background loop (opt-in via
  `MAIDAN_SCHEDULER_TICK_SECS`) that materializes a task thread for each schedule
  that comes due (Cluster 226). Each tick drains the due set: it **atomically
  claims and advances** a schedule (`Store::claim_next_due_schedule` — `FOR UPDATE
  SKIP LOCKED` on Postgres so concurrent replicas never double-fire one schedule;
  SQLite serializes writers) and then creates the task thread via
  `create_thread_with_event` + publishes it. A recurring schedule re-arms to `now +
  interval_secs` (fire-once-per-tick — no catch-up storm when far overdue); a
  one-shot deactivates. The claim commits before the thread is created, so a crash
  in between drops that firing (at-most-once) rather than duplicating it. Bounded to
  1000 firings/tick. New `maidan_task_schedules_fired_total{outcome}` metric.
  Disabled by default (unset env → the sweeper never starts).

## [226.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 10 — the
scheduled/recurring-task subsystem opens. No new gate tag.

### Added

- **Task-schedule store foundation.** A `maidan_task_schedules` table (pg 0038 /
  sqlite 0037) + `TaskSchedule` / `NewTaskSchedule` models + `TaskScheduleId`, and
  five `Store` methods (both backends): `create_task_schedule`, `get_task_schedule`,
  `list_task_schedules`, `delete_task_schedule`, and `due_task_schedules(now, limit)`
  (the sweeper's due-scan — active schedules with `next_run_at <= now`, oldest
  first). A schedule materializes a task thread when due: `interval_secs = NULL` is
  one-shot (fires once, then deactivates); a positive value is recurring (re-arm
  `next_run_at += interval_secs`). **No worker or routes yet** — a zero-blast-radius
  foundation (the Cluster 159 / 217 pattern); the background sweeper, REST, and MCP
  follow in later clusters.

## [225.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 9. No new
gate tag.

### Added

- **`get_queue_depth` MCP tool.** The MCP twin of `GET /channels/:cid/queue-depth`
  (Cluster 224): `{ channel_id }` → `{ open, ready, assigned, blocked }` over the
  shared `Store::channel_queue_depth`, so an MCP-only orchestrator can read a
  channel's task-queue depth to decide whether to scale workers. `workspace:read`;
  channel access enforced pre-dispatch (the `channel_id` arg). Full MCP 5-place
  wiring.

## [224.0.0] — 2026-08-17

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 8. No new
gate tag.

### Added

- **Channel task-queue depth.** `GET /channels/:cid/queue-depth` (`workspace:read`
  + channel access) → `{ open, ready, assigned, blocked }`: a point-in-time
  partition of a channel's open (non-terminal, non-tombstoned) task threads, so an
  orchestrator can decide whether to scale workers. `ready` uses the exact
  `claim_next` claimability predicate (unassigned or lease-expired, and every
  dependency terminal); `assigned` counts threads with a live, non-expired lease;
  `blocked` counts those waiting on a non-terminal dependency; the three partition
  `open`. One aggregate query per backend (`Store::channel_queue_depth`). Per-tenant
  on-demand DB aggregate, not a per-channel Prometheus label (the Cluster 188
  cardinality decision).

## [223.0.0] — 2026-08-16

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 7. No new
gate tag.

### Added

- **`wait_for_ready` MCP tool — block until a task becomes claimable.** The
  `wait_for_mention` analogue for the DAG: a long-poll (`workspace:read`) that
  subscribes to `ThreadReady` (Cluster 222) and returns the first ready task, or
  `null` on timeout (default 30 s, clamped 1 ms–300 s). Optional `channel_id` scopes
  it to one channel (access-checked pre-dispatch when present); otherwise it awaits
  any thread in the caller's workspace, RBAC-filtered per event via
  `can_access_thread` (a ready task in a private channel the caller can't see is
  skipped). Live-only — it sees readiness signalled after it subscribes, so pick up
  already-ready work with `claim_next_thread` first; `GET /mcp/stream`
  (`kinds=thread_ready`) is the resumable alternative. Full MCP 5-place wiring.

## [222.0.0] — 2026-08-16

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 6. No new
gate tag.

### Added

- **`ThreadReady` event — reactive task readiness.** When a thread transitions
  into a terminal state (closed/archived) and thereby unblocks its dependents, the
  transition route now publishes a `ThreadReady` event for each task that just
  became ready (all its dependencies terminal). This is the reactive counterpart to
  the pull-only `dependencies_satisfied` readiness query (Cluster 217) — an agent
  waiting on the DAG can subscribe (`kinds=thread_ready`) instead of polling. New
  `EventKind::ThreadReady` + `Event::ThreadReady { workspace_id, channel_id,
  thread_id, thread }`; a `Store::newly_ready_dependents(thread_id)` query (both
  backends) returns the non-terminal dependents now fully unblocked. Emitted only
  on a non-terminal → terminal edge (so a closed→archived move doesn't re-emit);
  best-effort (a failed emit never undoes the committed transition — readiness
  stays queryable). Derived + local-only: **not federatable** (a peer must not
  inject a readiness signal — each deployment computes its own).

## [221.0.0] — 2026-08-16

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 5. No new
gate tag.

### Changed

- **Transitive cycle prevention in the task DAG.** `add_thread_dependency` now
  rejects any edge that would close a cycle — direct (A→B then B→A) or transitive
  (A→B→C then C→A) — not just self-loops. Before inserting `thread_id →
  depends_on`, a recursive-CTE reachability walk from `depends_on` (following
  depends-on edges) checks whether `thread_id` is already reachable; if so the add
  fails with `InvalidInput` (REST `400`, MCP `InvalidParams`). The check and insert
  share a transaction so a concurrent add can't interleave. A cycle can never
  become ready — this closes a deadlock foot-gun rather than corruption. Both
  backends; no schema, route, tool, or contract change.

## [220.0.0] — 2026-08-15

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 4. No new
gate tag.

### Added

- **Task-dependency DAG MCP tools.** `add_thread_dependency` (`thread_id`,
  `depends_on_thread_id`; `thread:transition`) and `list_thread_dependencies`
  (`thread_id`; `workspace:read`, returns `{ dependencies, ready }`) — so an MCP
  agent can build and inspect the DAG, not just respect it (the readiness-aware
  `claim_next` from 218). The primary `thread_id`'s access is enforced by the
  pre-dispatch channel gate; `add_thread_dependency` additionally checks
  `ensure_thread_access` on the `depends_on` thread (the gate resolves only one id)
  plus a same-workspace guard. Full MCP 5-place wiring (handlers, dispatch,
  capability, gate, catalog, both `contracts/mcp-*.json`). The DAG's read/write
  surface is now complete over REST + MCP.

## [219.0.0] — 2026-08-15

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 3. No new
gate tag.

### Added

- **Task-dependency DAG management API.** Four REST routes on the thread surface:
  `POST /threads/:id/dependencies` `{ depends_on_thread_id }` adds an edge (`204`;
  `thread:transition`; both threads must share a workspace and be visible to the
  caller — cross-workspace or self dependency is `400`); `GET
  /threads/:id/dependencies` returns `{ dependencies, ready }` (the readiness flag —
  all deps terminal — rides the list; `workspace:read`); `DELETE
  /threads/:id/dependencies/:dep_id` removes an edge (`204`/`404`;
  `thread:transition`); `GET /threads/:id/dependents` lists the tasks blocked by
  this one (`workspace:read`). New DTOs `AddThreadDependency` /
  `ThreadDependenciesView`, OpenAPI paths + schemas, `http-capability-map` entries,
  and capability-matrix coverage. MCP dependency-management tools follow (agents can
  already *respect* the DAG via the readiness-aware `claim_next` from 218).

## [218.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Program B (agentic orchestration), part 2. No new
gate tag.

### Changed

- **`claim_next` is now readiness-aware.** The claim candidate query (the SQLite
  subquery and the Postgres `FOR UPDATE SKIP LOCKED` CTE, in both `claim_next` and
  the Cluster-209 `claim_next_with_event`, both backends) gains a `NOT EXISTS` clause
  that excludes any task with a non-terminal dependency. So the "pull the next task"
  primitive returns the oldest *ready* claimable task — an agent is never handed work
  blocked on an unfinished dependency, and picks it up once the dependency closes.
  Because the REST `POST /channels/:cid/threads/claim-next` route and the MCP
  `claim_next_thread` tool both call the same store method, this pure store change
  makes both dependency-aware with **no new API**. Dependency-free claiming is
  unchanged (`assignment_readside` regression green).

## [217.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). **Program B (agentic orchestration)** begins.
No new gate tag.

### Added

- **Task-dependency DAG (store foundation).** A new `maidan_thread_dependencies`
  table (both backends; pg 0037 / sqlite 0036) records directed edges — a thread
  (task) `thread_id` depends on `depends_on_thread_id` — with a self-loop `CHECK`
  and FK-cascade on both threads. New `ThreadDependency` model, a
  `ThreadState::is_terminal()` helper (closed/archived), and five store methods:
  `add_thread_dependency` (idempotent; self-dep rejected), `remove_thread_dependency`
  (conditional), `list_thread_dependencies` (what a task waits on),
  `list_thread_dependents` (what a task blocks), and `thread_dependencies_satisfied`
  (true iff every dependency is terminal — a task with no deps is ready). Landed as
  a zero-blast-radius foundation (no routes yet); the REST/MCP surface + a
  readiness-aware `claim_next` follow. Program B reuses the existing thread-as-task
  model (FSM + assignee/claim/lease from Clusters 171, 190–192).

## [216.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 15 (final): the RLS spike. No new gate tag.

### Changed

- **Row-Level Security assessed and deferred (decision ADR).** Program A's last
  item was a spike evaluating Postgres RLS as database-enforced tenant isolation
  beneath the app-layer RBAC. Outcome: **defer** — app-layer RBAC stays
  authoritative. A new `## Security` ADR in `docs/Decisions.md` records the RLS
  design (per-connection `SET LOCAL app.current_workspace` GUC + policies), the
  blockers (shared pool with no per-request tenant binding; workspace-agnostic
  `Store` trait; SQLite has no RLS → parity break; cross-workspace bearer
  orchestrator model; duplicates an already-comprehensive control), and the trigger
  conditions for revisiting. Docs-only. **With this, Program A (security &
  correctness round 2, Clusters 202–216) is complete.**

## [215.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 14: federation ingest trust policy. No new gate tag.

### Security

- **Federation ingest event-kind allowlist.** A federated peer's pushed (or
  pulled-and-ingested) events are now checked against `EventKind::federatable()` —
  an **allowlist-by-default** predicate (exhaustive `match`, so a new event kind
  won't compile until consciously classified). All collaboration-content kinds are
  federatable; **`ArtifactUpserted` is excluded** — federation replicates events,
  not artifact blobs, so an ingested `ArtifactUpserted` would announce a `sha256`
  whose bytes never arrive (a dangling reference / content-addressed existence
  oracle). Non-federatable kinds are rejected with `403` at `ingest_envelope`
  (covering both the push endpoint and the pull worker).
- **`MemberJoined` nested-workspace re-scope fix.** `remap_event_workspace`
  re-scopes an ingested event to the local peer workspace, and already remapped the
  nested `channel.workspace_id` for `ChannelCreated` — but `MemberJoined` passed its
  `member` through untouched, leaking the peer's *remote* `member.workspace_id` into
  the local view. It now re-scopes the nested member too.

## [214.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 13: transactional-outbox migration (references + artifacts — the last domain
mutations). No new gate tag.

### Changed

- **References + artifacts join the transactional outbox — completing the
  domain-mutation migration.** `add_reference_with_event` (`ReferenceAdded`) is
  scope-less like the creation events. `upsert_artifact_with_event(new,
  ref_workspace)` is the widest fold in the migration: the upload route did three
  dependent writes (`upsert_artifact` → conditional `record_artifact_ref`, the
  Cluster-204 per-workspace access link → `publish(ArtifactUpserted)`), now folded
  into **one transaction** (via a new `record_ref_in_tx`), preserving the upsert →
  ref → event ordering. `ref_workspace` is `Some(auth.workspace_id)` for a
  non-bypass caller (route computes `(!auth.bypass).then_some(auth.workspace_id)`);
  both the single-shot and multipart upload routes use it. This *strengthens*
  Cluster-204 isolation — the access ref now commits atomically with the upsert.
- **`publish()` correctly remains** (no rename/delete). Its remaining callers
  append **standalone events** with no domain-table row to be atomic with — the
  federation **relay** (`federation.rs`, re-publishing remote events onto the local
  bus) and `publish_routed_mentions` (fanning a durable `MentionRecorded` to each
  auto-parsed @mention for realtime routing). `publish()` = "durably append a
  standalone event + notify" is the right primitive for both, so the
  transactional-outbox refactor concludes at 214.

## [213.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 12: transactional-outbox migration (A2A ingest + member/workspace creation).
No new gate tag.

### Changed

- **A2A ingest + member/workspace creation join the transactional outbox.** The
  A2A ingest post reuses `post_message_with_event(new, None)` (it's the DM-post
  shape — a plain insert + `MessagePosted`, no post-insert edit). New
  `create_member_with_event` (`MemberJoined`) and `create_workspace_with_event`
  (`WorkspaceCreated`) commit the row **and** append its event in one transaction —
  the simplest `*_with_event` methods in the migration, since the created entity is
  the event's subject (no scope resolution). The routes call `*_with_event` +
  `publish_stored` (member/workspace are `#[cfg(feature = "bootstrap")]`; their
  gated `publish`/`Utc` imports go with the change). `publish()` now serves only the
  reference and artifact events (+ the federation relay).

## [212.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 11: transactional-outbox migration (message edit + tombstone). No new gate
tag.

### Changed

- **Message edit + tombstone join the transactional outbox.**
  `edit_message_with_event` (`MessageEdited`) and `tombstone_message_with_event`
  (`MessageTombstoned`) commit the mutation **and** append its event in one
  transaction. The edit SQL is extracted into a private `edit_in_tx` shared with
  Cluster 211's `edit_with_posted_event` (same mutation, different event).
  `tombstone_with_event` keeps the existing `NotFound`-on-no-op guard, so a
  re-tombstone appends no event. Both events carry `dm_conversation_id` (route
  parameter). The `edit_message` / `tombstone_message` routes call them +
  `publish_stored`; `message.rs` no longer calls `publish()` at all. `publish()`
  now serves only the A2A ingest post and the member / workspace / reference /
  artifact events (+ the federation relay).

## [211.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 10: transactional-outbox migration (regular message post). No new gate tag.

### Changed

- **The regular message-post path joins the transactional outbox.** The route now
  branches: a no-slash post uses `post_message_with_event` (insert + event in one
  tx, fully atomic); a slash post does a provisional insert, runs the (possibly
  external) slash-command dispatch, then a new `edit_message_with_posted_event`
  that commits the finalizing edit **and** its `MessagePosted` event (carrying the
  edited message) in one tx. Added `message_edits::append_in_tx` so the
  finalization records edit history on the same tx when the body changes.
- **`publish()` is not deleted.** The message-post hold-out is closed, but the
  outbox migration is larger than earlier notes implied: `publish()` still serves
  message edit/tombstone, the A2A ingest post, and the member / workspace /
  reference / artifact events, plus the federation relay (not a local domain
  write). Those migrate in follow-up clusters.

## [210.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 9: transactional-outbox migration (DM / group-DM posts). No new gate tag.

### Changed

- **DM / group-DM posts join the transactional outbox.** A new
  `post_message_with_event(new, dm_conversation_id)` store method inserts the
  message **and** appends its `MessagePosted` event in one transaction, resolving
  `(workspace, channel, thread)` via `message_scope_in_tx` and threading the
  caller-supplied `dm_conversation_id` (`Some` for a 1:1 DM, `None` for a group
  DM). The DM (`dm.rs`) and group-DM (`group_dm.rs`) post routes call it +
  `publish_stored`, dropping their hand-built `MessagePosted` literal and the
  now-redundant `resolve_thread_context`. The **regular** message-post path stays
  on `publish()` — it runs a slash-command edit after insert, so its event must
  reflect the final message (the last, entangled step of the refactor).

## [209.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 8: transactional-outbox migration (thread assignments). No new gate tag.

### Changed

- **Thread assignments join the transactional outbox.** `assign_thread`,
  `unassign_thread`, `claim_thread`, and `claim_next_thread` now have
  `*_with_event` store variants that commit the assignee change **and** append
  their `ThreadAssignmentChanged` event in one transaction, reusing Cluster 208's
  `events::thread_scope_in_tx` (a shared per-backend `append_assignment_event`
  builds the event). assign/unassign capture the **previous** assignee inside the
  same tx (a consistent read that replaces the route's separate `get_thread` —
  closing a read-then-write race on `previous_assignee_id`); claim/claim_next are
  conditional (`(result, Option<StoredEvent>)`), emitting only when the CAS
  actually claimed. The route's `publish_assignment` helper is removed;
  `renew_claim` is unchanged (no event). With this the thread-scoped batch is done;
  DM/group-DM posts and the slash-edit-entangled message post still use the
  retry-hardened `publish()`.

## [208.0.0] — 2026-08-14

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 7: transactional-outbox migration (thread transitions). No new gate tag.

### Changed

- **Thread FSM transitions join the transactional outbox.** `transition_thread`
  now has a `transition_thread_with_event` store variant that commits the state
  change **and** appends its `ThreadStateChanged` event in one transaction, over a
  new shared `events::thread_scope_in_tx` resolver (a thread's `(workspace,
  channel)`, the thread-scoped twin of the message-scoped resolver from 206). The
  existing FSM step is refactored into a private `transition_in_tx` core so
  `transition` (commit only) and `transition_with_event` (append event + commit)
  share one copy of the read → validate → HSM-parent-check → insert → update
  logic. The route (`thread.rs`) calls it + `publish_stored`. Migration continues:
  the assignment mutations (assign/unassign/claim/claim_next), DM/group-DM posts,
  and the entangled message-post path still use the retry-hardened `publish()`.

## [207.0.0] — 2026-08-13

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 6: transactional-outbox migration (pins + mentions). No new gate tag.

### Changed

- **Pins + mentions join the transactional outbox.** `pin_message`,
  `unpin_message`, and `record_mention` now have `*_with_event` store variants
  that write the row **and** append their `MessagePinned` / `MessageUnpinned` /
  `MentionRecorded` event in one transaction (Cluster 205 pattern), over the
  shared `events::message_scope_in_tx` resolver (pins need the channel too;
  mentions discard it). `unpin_message_with_event` is conditional — it returns
  `(removed, Option<StoredEvent>)` and appends `MessageUnpinned` only when a row
  was removed. Routes (`social.rs` pin/unpin, `message.rs` mention) call them +
  `publish_stored`. Migration continues: thread transitions/assignments,
  DM/group-DM posts, and the entangled message-post path still use the
  retry-hardened `publish()`.

## [206.0.0] — 2026-08-13

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 5: transactional-outbox migration (social mutations). No new gate tag.

### Changed

- **Votes + reactions join the transactional outbox.** `cast_vote`,
  `add_reaction`, and `remove_reaction` now have `*_with_event` store variants
  that write the row **and** append their `VoteCast` / `ReactionAdded` /
  `ReactionRemoved` event in one transaction (Cluster 205 pattern), with a shared
  `events::message_scope_in_tx` resolving the message's (workspace, channel,
  thread) in the same tx. `remove_reaction_with_event` appends the event **only
  when a row was actually removed** (idempotent no-op otherwise). The routes call
  the `*_with_event` variants + `publish_stored`. Behaviour unchanged; these
  events are now crash-consistent with their mutation. Pins + mentions migrate
  next; the remaining `publish()` callers shrink further.

## [205.0.0] — 2026-08-13

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 4: transactional outbox, **foundation** (multi-cluster refactor begins). No
new gate tag.

### Changed

- **Atomic domain-write + event-append (transactional outbox) — the pattern +
  first mutations.** A mutation committed its domain row in one transaction and
  then `publish()` appended the durable `Event` in a *separate* one, so a crash
  in that window committed the row but lost the event forever (no notification,
  delivery, or indexing). Cluster 184 only hardened this with a retry + loud
  metric. This cluster lands the real fix's foundation: a reusable
  `events::append_in_tx(&mut tx, event)` (both backends, extracted from
  `append`), and `create_channel_with_event` / `create_thread_with_event` store
  methods that insert the domain row **and** append its event (+ outbox row) in
  **one transaction** — they commit atomically or not at all. The routes call the
  `*_with_event` variants and a new `publish_stored` helper does the best-effort
  bus notification *after* the durable commit (a bus/relay hiccup can no longer
  undo a committed mutation). Behaviour is unchanged (the same events still reach
  the stream); only the crash-consistency guarantee is new. Remaining mutations
  (social/reactions/pins, thread transitions, and the slash-edit-entangled
  message-post path) migrate to `*_with_event` in follow-up clusters — until then
  they keep the retry-hardened `publish()` (a temporary, tracked mixed-atomicity
  during the migration).

## [204.0.0] — 2026-08-13

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 3. No new gate tag.

### Security

- **Cross-tenant artifact isolation.** Artifacts are content-addressed and
  **deduped across workspaces** (`maidan_artifacts` has no `workspace_id`), and
  `GET /artifacts/:sha` + `/artifacts/:sha/meta` gated only on `workspace:read` —
  so any caller who knew (or guessed) a SHA-256 could download another tenant's
  blob, and dedup was a known-plaintext oracle. A new `maidan_artifact_refs`
  table records which workspaces may access each SHA: a ref is written on upload
  (single-shot + multipart) to the uploader's workspace, and `get_artifact*` now
  requires a matching ref for the caller's workspace — returning **404** (not
  403) when absent, so a cross-tenant SHA can't even be confirmed to exist. Two
  workspaces that upload the *same* bytes each get their own ref and both keep
  access (dedup preserved). Migration backfills refs from each existing
  artifact's uploader's workspace. `bypass` (auth disabled) is unrestricted.
  Purge cleanup rides the ref FK's `ON DELETE CASCADE`; ref-counted blob GC
  (delete the shared blob only when no workspace still references it) is a
  documented follow-up.

## [203.0.0] — 2026-08-13

Post-gate hardening (Phase XXIV). Security & correctness round 2 (Program A) —
part 2. No new gate tag.

### Security

- **DM / group-DM participation on subscribe + metadata reads.** Cluster 180
  closed DM reads on the generic thread *route*, but the real-time **subscribe**
  path and the DM **metadata** routes still had no participant check:
  - **Subscribe (the leak of DM *content*):** `expand_event_filter` fetched a DM
    by a caller-supplied `dm_conversation_id` with no participant check and
    filled in its `thread_id`, so anyone with `event:subscribe` could tail any
    DM/group-DM's live messages on `GET /mcp/stream` (or WS) — by the
    `dm_conversation_id` *or* the `__dm__` `thread_id` directly. It now runs
    `ensure_thread_access` (DM-participant-aware, Cluster 180; bypass-exempt) on
    the resolved `thread_id`, closing both paths across WS + MCP-SSE.
  - **Metadata reads:** `GET /dm/:id` and `GET /group-dms/:id` returned a
    conversation's roster + thread to any workspace member; `list` enumerated any
    member's DM graph. A **session** caller must now be a participant to read a
    conversation and may only list its *own* (via the Cluster 202
    `ensure_acting_member` rule). A **bearer** is the orchestrator model and may
    still read/list on behalf of any member (unchanged); bypass unrestricted.

## [202.0.0] — 2026-08-12

Post-gate hardening (Phase XXIV). **Security & correctness round 2 — the new
four-program arc opens (from a 5-agent research sweep).** No new gate tag.

### Security

- **Session callers can no longer act as another member (anti-spoofing).** Only
  `post_message` pinned a **session** caller (browser/OIDC login, no API token)
  to its own member; every other member-attributed write trusted a
  caller-supplied `member_id`/`author_id`/`actor_id`/`editor_id` — so a `/ui`
  session user could post DMs / group-DM messages, edit messages, vote, react,
  pin/unpin, and transition/assign/claim/renew threads **as any member** in the
  workspace. The guard is now a shared `ensure_acting_member(auth, claimed)` and
  is applied on every member-attributed write surface. A **bearer token** is the
  orchestrator model and may still act as any member in its workspace
  (unchanged); `bypass` (auth disabled / tests) is unrestricted. The mention
  *target* (not an actor) and the assignee (a target, not the actor) are
  correctly left unguarded.

## [201.0.0] — 2026-08-12

Post-gate hardening (Phase XXIV). Performance & scale — arc D, part 4. No new
gate tag.

### Changed

- **Workspace-sharded event fan-out.** The event bus (both `InMemoryBus` and the
  `PostgresBus` local broadcast) used a single broadcast channel: every publish
  woke *every* subscriber, which then filter-and-discarded the events for other
  workspaces — O(total subscribers) per event regardless of relevance. A new
  `ShardedBroadcast` routes a publish only to the subscribers that could match
  it: the event's **workspace shard** plus a **global shard** for cross-workspace
  subscribers (operators, or any filter without a `workspace_id`). A
  workspace-scoped subscriber reads its workspace's shard and never sees another
  tenant's traffic. This is an optimization *under* the existing `EventFilter`
  (the filter still narrows by channel/thread/kind, just on far fewer events), so
  behavior is unchanged — a workspace-scoped filter never matched another
  workspace's events anyway. Shards are created lazily on first subscribe and
  pruned when their last receiver drops (bounded memory). Delivery, presence, and
  resource-notify (separate channels) are unaffected.

### Notes

- **Batched `pg_notify` declined** (Arc D item): the listener hydrates a single
  pointer per NOTIFY and the hot path publishes per-event with no natural batch
  to coalesce (only the latency-tolerant fallback relay batches), so a correct
  version needs range-hydration surgery on the delivery core for a win that only
  helps the non-hot path. **Read-replica routing deferred** — needs a read-pool
  threaded through `Store` + read-after-write handling and a real replica to
  validate. Both logged in Open Work.

## [200.0.0] — 2026-08-12

Post-gate hardening (Phase XXIV). Performance & scale — arc D, part 3. No new
gate tag.

### Changed

- **Filtered-ANN search: RBAC channel-deny pushed into the query.** Message
  search fetched the top-K hits and then dropped the ones the caller couldn't
  access (a per-thread post-filter). That both wasted work ranking inaccessible
  rows and — worse — **under-filled the requested `limit`**: ask for 10, get 4
  because 6 top hits were in a private channel. The server now computes the
  caller's private-channel deny-set (`maidan_auth::private_channel_deny_set` —
  private, non-DM channels the caller isn't a member of) and passes it as
  `SearchFilters::deny_channels`; both backends exclude those channels **in the
  query** (SQLite `NOT IN (…)`, Postgres `<> ALL($n)`), across lexical +
  semantic (hybrid composes them). So a full page of *accessible* hits comes
  back, and private-channel content is excluded at the source. The thread-level
  post-filter stays the authoritative, DM-participant-aware check (DMs live in
  the shared `__dm__` channel, so they're intentionally excluded from the
  channel-level pre-filter). Applied to REST `GET …/search` and the MCP
  `search_messages` tool; `bypass` callers get an empty deny-set (unchanged).

## [199.0.0] — 2026-08-12

Post-gate hardening (Phase XXIV). Performance & scale — arc D, part 2. No new
gate tag.

### Changed

- **Workspace-context pack builds its threads concurrently.**
  `build_workspace_context` built each page thread's context in a sequential
  `for` loop, and each `build_thread_context` is ~7 independent store
  round-trips — so a page of up to 50 threads stacked that latency linearly. It
  now builds them with a bounded-concurrency `buffered` stream
  (`CONTEXT_THREAD_CONCURRENCY = 8`), collapsing the wall-clock from
  `Σ per-thread` toward `ceil(N/8) ×` a single build while capping fan-out so one
  request can't saturate the connection pool. `buffered` preserves page order and
  short-circuits on the first error, so the response contract (and the
  tombstoned-mid-build 404) is unchanged — verified by the query-count guard
  (query *count* is identical, only concurrency changed) and a new
  no-cross-contamination correctness test.

## [198.0.0] — 2026-08-12

Post-gate hardening (Phase XXIV). Performance & scale — arc D, part 1 (the
baseline). No new gate tag.

### Added

- **Load / soak harness.** Arc D optimizes performance & scale (sharded fan-out,
  filtered-ANN search, batched context assembly), and those need a *baseline* to
  be measured against. `scripts/loadgen.sh` drives concurrent REST traffic (post
  message / read thread / search) at the server and reports per-op latency
  percentiles (min/mean/p50/p95/p99/max, ms) + overall throughput. The
  measurement is the `#[ignore]`d `load_baseline` test
  (`crates/maidan-server/tests/loadgen.rs`) — it never runs as a pass/fail CI gate
  (a hard latency floor would flake across runner hardware); it targets an
  in-process SQLite server by default, or a live/scaled deployment via
  `MAIDAN_LOADGEN_URL` + `_BEARER` + `_IDS`. Concurrency, per-worker iterations,
  and a timed-soak duration are env-tunable. The percentile math is a pure
  nearest-rank function that **is** unit-tested in CI. Baseline on the in-process
  path: ~1.8k ops/s at 6×20 with sub-10ms p99s.

## [197.0.0] — 2026-08-12

Post-gate hardening (Phase XXIV). Agentic task-queue depth — arc C, part 8 (the
finale). No new gate tag.

### Added

- **Tool-call transcripts.** Cluster 173 gave messages structured `content` with
  `ToolUse`/`ToolResult` blocks, but nothing correlated them — a multi-step
  agent's tool calls were scattered across message bodies. `tool_transcript`
  (maidan-types) walks a thread's messages, pairs every `ToolUse` with its
  `ToolResult` by id (order-independent — a result may land in a later message),
  and returns a `ToolTranscript`: the ordered tool calls each with `{name, input,
  result?}` and their message context, plus any `orphan_results` whose call is
  outside the scanned window. It's a **token-lean projection** — `Text`/`Code`
  blocks and `body` are dropped. Exposed as REST `GET /threads/:id/tool-transcript`
  and MCP `get_tool_transcript` (both `workspace:read`, thread-RBAC enforced,
  `limit` clamped 1..=500, default 200). Tombstoned messages are skipped.

### Notes

- **Arc C (agentic task-queue depth) is complete** (190 assignment read-side, 191
  MCP tools, 192 claim leases, 193 `list_roots`, 194 A2A `parts→content`, 195
  handoff notes, 196 `wait_for_mention`, 197 tool-call transcripts). Next: Arc D
  (performance & scale).

## [196.0.0] — 2026-08-12

Post-gate hardening (Phase XXIV). Agentic task-queue depth — arc C, part 7. No
new gate tag.

### Added

- **`wait_for_mention` — a blocking MCP long-poll for the next @mention.** An
  MCP-native agent can now *await* work instead of polling: `wait_for_mention`
  subscribes to the event bus filtered to the member's `MentionRecorded` events
  and blocks until one arrives (or a `timeout_ms` window lapses, default 30 s,
  clamped 1 ms–300 s), returning the mention event or `null` on timeout. It is a
  **live** primitive — it only sees mentions recorded after the call subscribes,
  so an agent drains existing ones with `get_inbox`/`list_mentions` first, then
  blocks for new ones; the resumable `GET /mcp/stream` SSE transport remains the
  at-least-once alternative. A mention in a private channel the caller can't
  access is filtered (RBAC via `can_access_thread`), so the tool never reveals
  activity in a thread the caller couldn't otherwise see. Requires
  `workspace:read`.

## [195.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Agentic task-queue depth — arc C, part 6. No
new gate tag.

### Added

- **Handoff notes on thread assignment.** `assign_thread` (REST
  `PUT /threads/:id/assignee` + the MCP tool) accepts an optional `note` — the
  free-text context an agent hands off with the work ("picked this up, blocked on
  the staging creds"). The note rides the `ThreadAssignmentChanged` event so the
  new assignee and every subscriber see it in real time. Event-only (not
  persisted on the thread): a handoff note is a moment-in-time message, and the
  assignment log already lives in the event stream. `#[serde(default,
  skip_serializing_if = "Option::is_none")]` keeps note-less assignments (claim /
  unassign / `claim_next`, which pass no note) byte-identical to before. The
  federation event-rewrite threads the note through unchanged.

## [194.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Agentic task-queue depth — arc C, part 5. No
new gate tag.

### Changed

- **A2A ingest preserves structured content.** The A2A ingress
  (`POST /a2a/v1/rpc`) built its message with `content: None`, joining the
  message's text parts into `body` and discarding the structure — so a message's
  ingress decided whether it carried structured `content` (REST/MCP could, A2A
  couldn't; Cluster 173). It now maps each text part to a `ContentBlock::Text`,
  so an A2A message carries the same structured content as a REST/MCP post.
  `body` stays the joined searchable projection (search/embeddings unchanged).

## [193.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Agentic task-queue depth — arc C, part 4. No
new gate tag.

### Added

- **`list_roots` MCP tool** — asks the connected client which roots
  (filesystem/workspace boundaries) it exposes, via the server→client
  `roots/list` request over `GET /mcp/streamable`. This is the first organic
  caller of `request_client`'s third verb (after sampling → `summarize_thread`
  and elicitation → `request_approval`). Requires a streamable session whose
  client declared the `roots` capability; returns the client's `{roots: [...]}`.
  Capability `workspace:read`.

## [192.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Agentic task-queue depth — arc C, part 3. No
new gate tag.

### Added

- **Claim leases + reclaim (dead-agent recovery).** A claimed thread can now
  carry a lease: `claim_next_thread` takes an optional `lease_secs` (REST body +
  MCP arg; omit for a durable claim), and a thread is claimable when it's
  unassigned **or** its lease has expired — so a claimed-then-dead agent no longer
  holds a thread forever; the next claimer transparently reclaims it (no reaper).
  New `POST /threads/:id/claim/renew` + MCP `renew_claim` extend the lease for the
  current assignee only (heartbeat). Adds a nullable `assignment_expires_at`
  column (migration pg 0035 / sqlite 0034) + `Thread.assignment_expires_at`.
  Manual `assign` / claim-a-specific-thread stay durable.

## [191.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Agentic task-queue depth — arc C, part 2. No
new gate tag.

### Added

- **MCP tools for the assignment read-side** (the deferred half of Cluster 190):
  `claim_next_thread` (atomically claim the oldest unassigned thread in a channel;
  channel access enforced pre-dispatch) and `list_assigned_threads` (a member's
  work queue; a member-scoped aggregate read, RBAC-filtered to threads the caller
  can access, like `search_messages`). An MCP-native agent can now discover and
  pull its work.

## [190.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Agentic task-queue depth — arc C, part 1. No
new gate tag.

### Added

- **Thread-assignment read-side** (Cluster 171 shipped only the write side).
  `GET /members/:id/assigned-threads` returns a member's work queue (live
  threads, oldest-first; RBAC-filtered to what the caller can access).
  `POST /channels/:cid/threads/claim-next` atomically claims the oldest
  unassigned thread in a channel for a member (returns the thread, or `null` when
  there's none) — Postgres uses `FOR UPDATE SKIP LOCKED` so concurrent claimers
  each get a distinct thread; a claim publishes `ThreadAssignmentChanged`. (MCP
  tools for these follow in the next cluster.)

## [189.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Multi-tenant SaaS operability — arc B finale.
No new gate tag.

### Added

- **Secret key rotation.** At-rest secrets (federation peer tokens, webhook /
  slash / fsm-hook secrets) were AEAD-encrypted with a single key from
  `FEDERATION_ENCRYPTION_KEY` with no rotation path — changing it stranded every
  stored ciphertext. A try-all-keys decrypt keyring now lets you rotate: set the
  new key as `FEDERATION_ENCRYPTION_KEY` and move the old key(s) into
  `FEDERATION_DECRYPT_KEYS` (comma-separated, same encoding). Encryption always
  uses the new primary; decryption tries the primary then the fallbacks. No
  ciphertext-format change (backward-compatible); AEAD authentication makes
  trying keys safe. A malformed `FEDERATION_DECRYPT_KEYS` entry fails startup
  rather than silently stranding a key.

## [188.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Multi-tenant SaaS operability — arc B, part 4.
No new gate tag.

### Added

- **Per-workspace usage / metering.** `GET /workspaces/:id/usage` (gated on
  `workspace:read`) returns live member/channel/thread/message counts (excluding
  tombstoned rows) for one workspace — a metering / quota-visibility basis that
  stays low-cardinality (a per-request DB aggregate, not a per-tenant Prometheus
  series, which would blow up cardinality as tenants grow). Artifact storage
  bytes are intentionally omitted (blobs are content-addressed and deduped across
  workspaces, so per-tenant bytes is ill-defined).

## [187.0.0] — 2026-08-11

Post-gate hardening (Phase XXIV). Multi-tenant SaaS operability — arc B, part 3.
No new gate tag.

### Added

- **Workspace export / portability.** `GET /workspaces/:id/export` (gated on
  `token:admin`) returns the workspace's content graph as one JSON bundle —
  workspace, members, channels (+ members), threads, messages (+ edits, paginated
  to completeness), pins, and references — so a tenant can be migrated or archived,
  not only deleted. DM/group-DM message content is included (DM threads live in
  the `__dm__` channel). **Excludes secrets** (API tokens, webhook/slash/OIDC) and
  operational tables (events, audit, deliveries). Reactions/votes and artifact
  blobs are not yet included, and there is no import path yet (see Open Work).

## [186.0.0] — 2026-08-10

Post-gate hardening (Phase XXIV). Multi-tenant SaaS operability — arc B, part 2.
No new gate tag.

### Added

- **Opt-in data-retention pruning** for the unbounded-growth tables. A background
  sweeper deletes rows past a per-table age:
  `MAIDAN_RETENTION_EVENTS_DAYS` / `_AUDIT_DAYS` / `_DELIVERIES_DAYS` (unset/`0` =
  keep forever), every `MAIDAN_RETENTION_SWEEP_SECS` (default daily), in batches
  of `MAIDAN_RETENTION_BATCH` (default 5000) so a first sweep doesn't take one
  giant lock. Exposes `maidan_retention_pruned_total{table}`.
  - **Event-log safety:** events are pruned only up to `min_delivery_cursor` (the
    lowest watermark across all at-least-once consumers), so a lagging durable
    consumer never loses an undelivered event; with no such consumer, prune by age
    alone.
  - **Deliveries:** only terminal (delivered/quarantined) rows are eligible;
    in-flight rows are never pruned.

## [185.0.0] — 2026-08-10

Post-gate hardening (Phase XXIV). Multi-tenant SaaS operability — arc B, part 1.
No new gate tag.

### Changed

- **Helm liveness no longer restart-storms on a degraded dependency.** Both
  probes hit `/health`, which returns `503` when any dependency (DB/storage/
  indexer/bus) is degraded — so a transient DB blip failed the *liveness* probe
  and Kubernetes killed the pod mid-recovery. Liveness + a new startupProbe now
  hit the shallow `/health/live` (always `200`, process-alive); readiness hits
  the deep `/health/ready` (the same check as before). Probe timings are tunable
  via `.Values.probes`.

### Added

- **`PodDisruptionBudget`** template (opt-in; enabled with `minAvailable: 1` in
  `values-prod.yaml`) so node drains/rollouts keep a pod serving.
- **`NetworkPolicy`** template (opt-in, safe-by-default: ingress restricted to
  the HTTP port, egress open with DNS always allowed; tighten via `ingressFrom`
  / `allowAllEgress: false` + `egress`).
- **`existingSecret`** — reference a pre-created Secret instead of rendering one
  from `.Values.secrets`, keeping secret material out of values files / release
  history.

## [184.0.0] — 2026-08-10

Post-gate hardening (Phase XXIV). Security & correctness — arc A finale. No new
gate tag.

### Changed

- **Domain events are no longer silently lost when the log append fails.** Every
  mutation commits its domain row, then `publish()` appends the `Event` in a
  separate transaction; the old code logged a single `warn` and dropped the event
  on append failure while the caller still got a `2xx` — no notification, no
  delivery, no indexing. `publish()` now retries the durable append on transient
  errors (3 attempts, 50 ms backoff), distinguishes an append failure (dangerous
  — event lost) from a benign bus-publish failure (already logged), and on a hard
  failure logs `event.append_failed` and increments the new
  `maidan_event_append_failures_total` metric so a lost event is alertable.

### Notes

- This hardens the dual write; it is **not** full single-transaction atomicity
  (a crash between the domain commit and a successful append still loses the
  event). The transactional-outbox refactor that would close that is a larger,
  tracked follow-up (see Open Work).

## [183.0.0] — 2026-08-10

Post-gate hardening (Phase XXIV). Security & correctness — arc A, part 5. No new
gate tag.

### Added

- **Default-on global rate limit.** When `MAIDAN_RATE_LIMIT_MAX` is unset the
  server now applies a built-in per-client floor (1200 requests / 60 s per
  bearer/IP), so a deployment that configures nothing still has a DoS floor. An
  explicit `MAIDAN_RATE_LIMIT_MAX` (including `0` to disable) always overrides.
  The per-workspace fairness limit stays independently opt-in. (Library
  embedders/tests are unaffected — the default is only enabled by the server
  binary.)
- **Explicit, tunable request body-size cap** via `MAIDAN_MAX_BODY_BYTES`
  (default 2 MiB, matching axum's previously-implicit extractor limit). Oversized
  request bodies now return `413 Payload Too Large` (`problem+json`) instead of a
  flattened `400`.

## [182.0.0] — 2026-08-10

Post-gate hardening (Phase XXIV). Security & correctness — arc A, part 4. No new
gate tag.

### Added

- **Audit-trail coverage for credential + membership mutations.** The audit log
  (`GET /workspaces/:id/audit`, `GET /operator/audit`) now records `token.mint`,
  `token.revoke` (including the OIDC first-admin session mint), `app_token.mint`,
  `app_installation.revoke`, `channel_member.add`, `channel_member.remove`, and
  `message.purge` — previously these security-critical state changes left no
  trace. Each row carries the actor, a `target_kind`/`target_id`, and metadata
  (workspace, subject member, capabilities). Writes are best-effort (a failed
  audit insert logs `audit.write_failed` and does not break the operation — a
  mint must never lose its secret to an audit hiccup).

### Notes

- Table-level 401/403 **denial** auditing was deliberately *not* added: a
  rejected, attacker-controlled request stream would be an unbounded audit-table
  write amplifier. Denials stay in structured logs + metrics.

## [181.0.0] — 2026-08-10

Post-gate hardening (Phase XXIV). Security & correctness — arc A, part 3. No new
gate tag.

### Changed

- **One `EventKind` wire-form parser instead of three.** The store kept its own
  `parse_kind` copy in each of `postgres/events.rs` and `sqlite/events.rs`,
  duplicating `maidan_types::EventKind::parse`. `append` re-parses the `kind`
  column on read-back, so a store copy missing a variant made the insert **fail
  after INSERT and silently roll back** (the Cluster 171 bug —
  `thread_assignment_changed` was in the enum's `parse` but not the store
  copies). Both store copies now delegate to the single `EventKind::parse`, so
  there is no per-backend mapping to drift.

### Added

- `EventKind::ALL` + a round-trip guard (`parse(as_str())` for every variant)
  with a compile-time tripwire: adding a variant fails the guard test's
  exhaustive match until it's listed. `EventKind` is now `Copy` (fieldless enum).

## [180.0.0] — 2026-08-10

Post-gate hardening (Phase XXIV). Security & correctness — arc A, part 2. No new
gate tag.

### Security

- **DM/group-DM threads are now participant-checked on every surface.** DM
  threads live in the shared `__dm__` channel, which `ensure_channel_access`
  exempts — so the generic content routes (`GET /threads/:id`, `…/messages`,
  `…/context`, plus message/reaction/pin/vote routes and the A2A ingress) let any
  workspace member read/write a DM they weren't part of, and workspace **search +
  workspace-context leaked DM message content** to non-participants.
  `ensure_thread_access` is now DM-participant-aware (via a new
  `ensure_dm_participant`), all thread/message-scoped routes gate on it, and the
  search/context filters key on per-thread access (`can_access_thread`) instead
  of the channel. Dedicated `/dm` routes, participants, and public/private
  channels are unchanged.

## [179.0.0] — 2026-08-10

Post-gate hardening (Phase XXIV). Security & correctness — new program, arc A,
part 1. No new gate tag.

### Security

- **A2A JSON-RPC ingress now enforces channel/thread access.** `POST /a2a/v1/rpc`
  previously gated only on the `message:post` capability + workspace, so an
  external A2A agent could post into — and read tasks whose context thread lives
  in — a **private channel it isn't a member of**. This was the one surface the
  160–165 RBAC arc missed. Both the write (`SendMessage`) and read (`tasks/get`)
  paths now call `ensure_channel_access`, identical to REST/MCP. (`__dm__`
  generic-route tightening follows in the next cluster.)

## [178.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Token efficiency — arc 4 (round 3), part 4
(final). No new gate tag.

### Added

- **Opt-in lean event frames.** A `lean` subscribe flag (WS subscribe frame /
  MCP-SSE query param) makes the streamed domain-event frames carry only
  `{log_id, kind, workspace_id?, channel_id?, thread_id?, member_id?}` — a
  "something happened, go fetch" pointer — instead of the full serialized event.
  Saves tokens for agents that tail for activity and read on demand. Default off;
  the lean frame is a strict subset of the full frame's top-level fields, so
  `log_id`/`kind`/`thread_id`-based client logic is unchanged. Applies on all
  delivery paths (optimistic live, lag-replay, at-least-once reconcile).

This completes token round 3 (175–178) and the post-v155 four-arc program
(enterprise hardening 156–165, perf + CI/CD 166–170, agentic features 171–174,
token round 3 175–178).

## [177.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Token efficiency — arc 4 (round 3), part 3. No
new gate tag.

### Changed

- **Empty message metadata is omitted from the wire.** `Message.metadata` now
  serializes with `skip_serializing_if` when it's empty (`{}`/`null`), so every
  serialized message (REST responses, event frames, MCP tool results,
  write-acks) drops the ubiquitous `"metadata":{}`. Serialization-only and
  idempotent — the stored column is unchanged, a wire message without `metadata`
  deserializes back to an empty object, and consumers already tolerate absence
  (`/ui` metadata readers are null-guarded). Mirrors the `content` omit-empty
  from Cluster 173.

## [176.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Token efficiency — arc 4 (round 3), part 2. No
new gate tag.

### Changed

- **`tools/list` is capability-filtered.** The MCP tool list now returns only
  the tools the caller's token capabilities allow (via `tools::catalog_for`),
  instead of the entire catalog — a capability-scoped agent no longer pays tokens
  for ~40 tool schemas it can't invoke. Bypass / full-capability callers see the
  full list, unchanged. The unfiltered catalog (contract tests, full-cap callers)
  is untouched — only the per-caller response is scoped.

## [175.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Token efficiency — arc 4 (round 3), part 1. No
new gate tag.

### Added

- **`snippet_only` on the MCP `search_messages` tool** (default `false`): drops
  the full message `body` from each hit and keeps only the snippet, saving tokens
  in agent search results — parity with the REST `snippet_only` param (Cluster
  152), reusing the same `SearchHit::into_snippet_only`.

## [174.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Agentic features — arc 3, part 4 (final). No
new gate tag.

### Added

- **Human-in-the-loop approvals.** A new MCP `request_approval` tool lets an
  agent ask the human on the connected client to approve or reject an action,
  via a server→client `elicitation/create` over the GET `/mcp/streamable` stream
  (requires the client to have declared the `elicitation` capability). It
  returns `{approved, action, content}` — `approved` is true iff the human chose
  `accept`; `decline`/`cancel`/timeout mean not approved (fail-closed). The
  elicitation analogue of the sampling-backed `summarize_thread`.

Arc 3 (agentic features) is complete: thread task assignment/handoff (171), MCP
structured backpressure (172), structured message content (173), HITL approvals
(174).

## [173.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Agentic features — arc 3, part 3. No new gate
tag.

### Added

- **Structured message content.** Messages can now carry an ordered list of
  typed `content` blocks — `text`, `code`, `tool_use`, `tool_result`,
  `resource_link` (internally tagged, matching the MCP/Anthropic dialect) — over
  both REST (`POST`/`PATCH` message `content`) and MCP (`post_message`,
  `edit_message`, `post_dm_message`). Persisted in a new nullable column
  (Postgres JSONB / SQLite JSON). When `content` is posted without a `body`, the
  server derives `body` from the text-bearing blocks, so full-text + semantic
  search are unchanged (a `tool_use` block contributes nothing to `body`). Plain
  body-only messages have `content: null`. Tombstone + workspace-purge clear it.

### Notes

- Federation/A2A-ingested messages remain body-only for now (the ingest path
  doesn't yet map `parts → content`) — logged in Open Work. No new event kind,
  capability, MCP tool name, or contract change.

## [172.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Agentic features — arc 3, part 2. No new gate
tag.

### Added

- **Structured backpressure for MCP clients.** A rate-limited `POST /mcp` or
  `POST /mcp/streamable` now returns a JSON-RPC error envelope — code `-32029`
  with `data.retry_after_ms` — instead of only an opaque transport 429, so an
  agent's JSON-RPC layer gets a typed, machine-readable backoff signal. The
  response is still HTTP 429 with a `Retry-After` header (HTTP infra still sees
  the backpressure); non-MCP routes keep the existing `problem+json` body. The
  per-token-capability quota limiter shares this path. New
  `McpError::RateLimited { retry_after_ms }`.

## [171.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Agentic features — arc 3, part 1. No new gate
tag.

### Added

- **Thread task assignment / handoff.** Threads gain an `assignee_id` axis
  (orthogonal to the state FSM) so agents can own work. New operations, all
  gated by the existing `thread:transition` capability + per-channel RBAC:
  - REST: `PUT /threads/:id/assignee` (assign/handoff), `DELETE
    /threads/:id/assignee` (unassign), `POST /threads/:id/assignee/claim`.
  - MCP tools: `assign_thread`, `claim_thread`, `unassign_thread`.
  - **Atomic claim**: `claim` is a compare-and-set (`WHERE assignee_id IS NULL`)
    so exactly one of N concurrent claimers wins; it returns `{thread, claimed}`
    rather than erroring on a loss.
  - Every change emits a `ThreadAssignmentChanged` event on the bus (prev→new
    assignee + actor), so orchestrators see ownership changes live.

### Fixed

- **`release.yml` trivy job** now pins `aquasecurity/trivy-action@v0.36.0`
  (Cluster 170 used `@v0.28.0`, whose internal `setup-trivy@v0.2.1` pin was
  removed upstream — it failed to resolve on the v170.0.0 release run). v0.36.0
  pins its dependency by commit SHA.

## [170.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). CI/CD — arc 2, part 5 (closes arc 2). No new
gate tag.

### Changed

- **The arm64 release image builds on a native runner.** `release.yml` built the
  `linux/arm64` `maidan-server` image under QEMU emulation on an amd64 runner;
  because the server Dockerfile does a full `cargo build --release`, that leg ran
  ~2 h and dominated the ~2 h 18 m release. Each matrix leg now builds only its
  native platform (`ubuntu-latest` for amd64, `ubuntu-24.04-arm` for arm64), and
  the QEMU setup step is removed. (`maidan-postgres` is unchanged — its image is
  `FROM pgvector/pgvector` with no compile, so its emulated arm64 build is fast.)

### Added

- **Container image vulnerability scan (trivy).** A new `trivy-scan` release job
  scans the published `maidan-server` image for fixable OS + library
  `CRITICAL,HIGH` CVEs. Report-only on introduction (does not gate the release);
  promotable to blocking once the baseline is reviewed.

## [169.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Perf — arc 2, part 4 (closes the DB-hot-path
items). No new gate tag.

### Changed

- **Optimistic-path delivery-cursor writes are coalesced.** The optimistic live
  subscribe path (`forward_bus_items`) issued an `advance_delivery_cursor` DB
  UPSERT on **every** delivered event — one write per event per subscriber, in
  the hot path. It now buffers the highest delivered `log_id` and persists it at
  most once per 64 events or 500 ms, plus a flush when the stream ends. The
  lag-replay path advances once to the batch high-water instead of per row.
  Safe because this cursor is best-effort (the authoritative at-least-once path,
  `reconcile_deliver`, already batches), `advance_delivery_cursor` is monotonic,
  and delivery is at-least-once — a coalesced-away write only re-delivers a few
  already-seen events on reconnect, never skips.

## [168.0.0] — 2026-08-07

Post-gate hardening (Phase XXIV). Perf/correctness — arc 2, part 3. No new gate tag.

### Changed

- **Outbox relay does fewer round-trips per row.** `list_pending` now JOINs the
  event payload from `maidan_events`, so the relay publishes straight from the
  pending row instead of a per-row `get_stored_event`, and the
  successfully-published rows are marked in a single `mark_published_batch`
  after the loop rather than one `UPDATE` each. A full 64-row batch drops from
  ~128 extra DB calls to ~1. The at-least-once contract is unchanged (a crash
  between publish and the batch mark re-publishes the batch; consumers dedup on
  `log_id`).
- **Broadcast-channel capacity is env-tunable.** The event bus and the
  presence/resource notifiers read `MAIDAN_BUS_BROADCAST_CAP` (default 1024) via
  a shared `maidan_bus::broadcast_cap_from_env()`, replacing three hard-coded
  `1024` constants. A larger cap lets a slow subscriber lag further before the
  channel drops the oldest frames.

### Fixed

- **Removed two `unwrap()`s in `webhook_worker.rs`.** The Cluster 166
  lazy-payload change left `payload.as_deref().unwrap()` in library code — a
  CLAUDE.md violation that the `lint` job's dedicated `-D clippy::unwrap_used`
  step rejects. It merged during the GitHub Actions outage (validated only with
  `--all-targets -D warnings`, which does not enable that restriction lint), so
  `main` went red once CI recovered. Rewritten with `let-else` / `if let Some`.

## [167.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Perf/correctness — arc 2, part 2. No new gate tag.

### Fixed

- **Rate-limiter in-memory bucket map is now bounded.** Entries were never
  evicted — the map grew without bound as distinct keys (tokens/clients/routes ×
  windows) accumulated: a memory leak. It now sweeps entries whose window has
  fully elapsed once the map crosses a threshold (`MEMORY_SWEEP_THRESHOLD`).

### Changed

- **Embedding upserts cache the model→table resolution.** `PostgresSearch` now
  caches `model → table_name`, so a steady-state `upsert_embedding` skips the
  `maidan_embedding_models` SELECT + `CREATE TABLE IF NOT EXISTS` checks that ran
  on every call — halving the round-trips in the live indexer + reindex hot path.

## [166.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Perf/correctness — arc 2, part 1. No new gate tag.

### Fixed

- **SQLite `foreign_keys`/`busy_timeout` now apply to every pooled connection.**
  They were run once on a single pooled connection, but both are *per-connection*
  in SQLite — so the other connections ran with FK enforcement **off** (data-
  integrity risk) and fail-fast-on-`SQLITE_BUSY`. They (and `journal_mode = WAL`)
  now run in the pool's `after_connect` hook (`sqlite_pool_options_with`).
- **Webhook fan-out no longer scans every workspace's subscriptions per event.**
  `enqueue_matches` listed **all** enabled webhook subscriptions across all
  workspaces on every bus event and filtered in memory; it now queries only the
  event's workspace (`list_enabled_webhook_subscriptions_for_workspace`, using
  `idx_webhook_subs_workspace`) and builds the payload lazily on first match.

## [165.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Channel/thread RBAC, part G — reference authorization (arc complete). No new gate tag.

### Security

- **References are now access-controlled.** `create_reference` /
  `list_references` (REST) and `add_reference` (MCP) had **no** workspace or
  channel check — a token could link or list references into any thread/message,
  including private channels, cross-tenant. They now resolve each referenced
  Thread/Message via `ensure_thread_access` / `ensure_message_access` (which also
  enforces the workspace), closing the last RBAC gap. **With 159–165 the
  channel/thread RBAC arc is complete**: private-channel access is enforced on
  read/write (REST+MCP), events (WS+MCP SSE), management (`channel:admin`), and
  references.

## [164.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Channel/thread RBAC, part F — the `channel:admin` membership-management API. No new gate tag.

### Added

- **`channel:admin` capability + channel-membership management API.** New
  capability (in `KNOWN`, not `default_minted`). REST: `POST` / `GET`
  `/channels/:cid/members` and `DELETE /channels/:cid/members/:mid` (add-or-update
  role / list / remove), gated by `channel:admin`. MCP: `add_channel_member` /
  `list_channel_members` / `remove_channel_member` tools. OpenAPI-documented;
  wired into the HTTP + MCP capability maps and matrices. This makes private
  channels operational — admins can grant/revoke access, not only the creator's
  auto-add. End-to-end e2e: add member → access granted → list → remove →
  denied.

## [163.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Channel/thread RBAC, part E — verified subscribe grants. No new gate tag.

### Security

- **WS/MCP subscribe grants are now verified against membership.**
  `apply_subscribe_grants` previously trusted the client's asserted
  `channel_grants`, so a non-member could subscribe with a private channel's id
  and receive its events. It now drops any asserted private-channel grant the
  caller isn't a member of (public + `__dm__` pass; bypass unchanged), so the
  channel is denied and lands in `private_channel_deny`. The WS subscribe path
  (`ws.rs`) resolves the caller's identity *before* applying grants; the MCP SSE
  stream passes its `AuthContext` through. Closes the private-channel **event**
  leak on the WebSocket + MCP SSE surfaces.

### Not yet covered (follow-ups)

- `reference.rs` authorization and the `channel:admin` membership API remain
  (Open Work).

## [162.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Channel/thread RBAC, part D — MCP aggregate-read filtering. No new gate tag.

### Security

- **MCP aggregate reads no longer return private-channel content to non-members.**
  `search_messages` drops hits in inaccessible channels; `list_channels` hides
  private channels the caller isn't in (public + `__dm__` always listed);
  `get_workspace_context` drops packed threads in inaccessible channels. Each
  caches the per-channel decision. Together with 160 (REST) and 161 (MCP
  point-access), the channel-content read/write vuln is now closed on both
  primary surfaces.

### Not yet covered (follow-ups)

- The WebSocket event-subscribe private-channel gate (`subscribe_grants`),
  `reference.rs` authorization, and the `channel:admin` membership API remain
  (Open Work).

## [161.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Channel/thread RBAC, part C — MCP point-access enforcement. No new gate tag.

### Security

- **MCP tools now enforce per-channel access.** A pre-dispatch gate
  (`tools::dispatch`) resolves each point-access content tool's target and calls
  `ensure_channel_access` / `ensure_thread_access` / `ensure_message_access`:
  `list_threads`, `list_messages`, `post_message`, `get_thread_context`,
  `summarize_thread`, `pin_message`/`unpin_message`/`list_pins`, `edit_message`,
  `record_mention`, `cast_vote`, `add_reaction`/`remove_reaction`/`list_reactions`.
  `resources/read` also gates the `maidan://threads/{id}` and
  `maidan://channels/{id}` resources. Bypass callers pass; DM tools rely on their
  own participant checks. Closes the MCP read/write path into private channels.

### Not yet covered (follow-ups)

- MCP **aggregate** reads still return private content — `search_messages`,
  `get_workspace_context`, `list_channels` — filtered in the next cluster. The
  WS event-subscribe gate, `reference.rs`, and the `channel:admin` membership API
  also remain (Open Work).

## [160.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Channel/thread RBAC, part B — REST enforcement. No new gate tag.

### Security

- **Private channels are now access-controlled over REST.** New
  `ensure_channel_access` / `ensure_thread_access` / `ensure_message_access` /
  `can_access_channel` in `maidan-auth`: a public channel is open to the whole
  workspace; a **private** channel requires a `channel_members` row. Enforced on
  every REST content surface — channels (get/list, and `create` auto-adds the
  creator as an admin of a new private channel), threads (create/list/get/
  context/transition), messages (post/list/get/edit/tombstone/purge/mention/
  edits), reactions/pins/votes, workspace-search hits, and the workspace-context
  pack. Closes the reported gap where any `message:post` token could read or
  write **any** channel in its workspace, including private ones. The `__dm__`
  system channel is exempt (DM/group-DM membership is enforced per-conversation).
  Public channels and DMs are unchanged.

### Not yet covered (follow-ups)

- MCP tool enforcement (Cluster 161), the WebSocket event-subscribe
  private-channel gate (`subscribe_grants` verification), and `reference.rs`
  authorization remain — tracked in Open Work.

## [159.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Channel/thread RBAC, part A — membership model (no enforcement). No new gate tag.

### Added

- **`channel_members` membership model** — new table (postgres `0032` / sqlite
  `0031`): `(channel_id, member_id, role ∈ {member, admin}, created_at)`. New
  `ChannelMember` / `ChannelMemberRole` types and four `Store` methods
  (`add_channel_member` idempotent upsert / `remove_channel_member` /
  `list_channel_members` / `channel_is_member`), both backends. This is the
  substrate for per-channel authorization; **no enforcement yet** (Cluster 160)
  — public channels remain open to the workspace, so there is no behavior change.

## [158.0.0] — 2026-08-06

Post-gate hardening (Phase XXIV). Enterprise-hardening arc, part 3 — signed container images. No new gate tag.

### Added

- **Keyless cosign signatures on the container images.** A new `sign-images`
  release job resolves each pushed tag (`maidan-server`, `maidan-postgres`) to
  its immutable index digest and `cosign sign`s the digest via the workflow's
  GitHub OIDC identity (no private key) — the same trust root as the existing
  release-blob signatures. Admission controllers (Kyverno / Sigstore policy) can
  now verify the images; `docs/Operations.md` documents the `cosign verify`
  command. (trivy image scanning is deferred to the perf/CI arc.)

## [157.0.0] — 2026-08-05

Post-gate hardening (Phase XXIV). Enterprise-hardening arc, part 2 — fail-closed auth. No new gate tag.

### Security

- **`AUTH_DISABLED` is now fail-closed.** It was rejected only when
  `MAIDAN_ENV=production`, so any non-production or `MAIDAN_ENV`-unset deployment
  that set `AUTH_DISABLED=1` served every request unauthenticated. It now takes
  effect **only** when the explicit **`MAIDAN_ALLOW_INSECURE_NO_AUTH=1`**
  acknowledgement is also set, and never in production — a stray `AUTH_DISABLED=1`
  refuses boot (`validate_insecure_no_auth`, enforced in `Config::from_env` and
  again in `auth_disabled_from_env()` as defense-in-depth) instead of silently
  disabling auth.

### Changed

- Dev/test/CI manifests that run without auth (`compose.yaml` all profiles,
  `helm/maidan/values-ci.yaml`) now set `MAIDAN_ALLOW_INSECURE_NO_AUTH`
  alongside `AUTH_DISABLED`. `docs/Production.md` + `docs/Threat-Model.md` (T2)
  updated.

## [156.0.0] — 2026-08-05

Post-gate hardening (Phase XXIV). Enterprise-hardening arc, part 1 — production-safety defaults. No new gate tag.

### Added

- **SIGTERM graceful shutdown.** The server now drains on `SIGTERM` as well as
  `SIGINT` (unix). Kubernetes/systemd send `SIGTERM` on rollout/stop; previously
  the process was killed mid-request instead of draining through
  `with_graceful_shutdown` + the worker `shutdown()` sequence. Falls back to
  `SIGINT`-only if the handler can't be installed; non-unix unchanged.

### Changed

- **`MAIDAN_DB_STATEMENT_TIMEOUT_MS` now defaults to `30000` (30 s)** instead of
  `0` (disabled), so a runaway query can't pin a pooled connection indefinitely.
  Boot migrations remain exempt (they reset `statement_timeout = 0` under the
  advisory lock); set `0` to restore the uncapped behavior. `docs/Production.md`
  updated.

## [155.0.0] — 2026-08-05

Post-gate hardening (Phase XXIV). First organic `request_client` caller — sampling-backed `summarize_thread` (arc lane 3, part 2). No new gate tag.

### Added

- **`summarize_thread` MCP tool** — the first organic caller of
  `request_client`. A `tools/call` gathers the thread transcript and issues a
  server→client `sampling/createMessage` over the canonical GET stream (the
  Cluster 154 delivery path), returning the client's completion. Requires a
  streamable session whose client declared the `sampling` capability;
  `workspace:read`. `limit` clamped `1..=500`, optional `instructions`.

### Changed

- **Tool dispatch carries the streamable session id.** `McpServer::handle` now
  delegates to `handle_in_session(request, auth, session_id)`; `dispatch` /
  `tools_call` / `tools::dispatch` thread an optional `Mcp-Session-Id` so a tool
  can target its client. The `POST /mcp/streamable` JSON-accept path and both
  SSE session paths pass the session through; non-streamable transports pass
  `None`.

## [154.0.0] — 2026-08-05

Post-gate hardening (Phase XXIV). `request_client` GET-stream delivery fix (arc lane 3, part 1). No new gate tag.

### Fixed

- **Server→client requests now reach the canonical `GET /mcp/streamable`
  stream.** `request_client` (sampling / roots / elicitation) previously pushed
  onto the session's POST-leg mpsc, so a client listening on the spec-canonical
  server→client GET stream never received them — only a client holding a POST
  SSE leg did. A new per-session broadcast (`push_client_request` /
  `subscribe_client_requests`) delivers server→client requests; `stream_get`
  merges them with the unsolicited notifications. The POST-leg response/
  notification mpsc and the replay log are untouched.

### Changed

- Server→client requests are delivered on the **GET stream**, not the POST leg
  (spec-canonical). `request_client` has no organic caller yet (one arrives in
  Cluster 155), so no integration regresses.

## [153.0.0] — 2026-08-05

Post-gate hardening (Phase XXIV). Live-updating `/ui` thread view (UI polish). No new gate tag.

### Added

- **Live thread view in the `/ui` console** — WebSocket domain-event frames
  whose `thread_id` matches the open thread now refresh the message list
  (debounced, ≤1 reload / 300 ms) instead of only appearing as `[log_id] kind`
  log lines in the Events tab. Triggers on the thread-content kinds
  (`message_posted` / `message_edited` / `message_tombstoned` /
  `reaction_added` / `reaction_removed` / `message_pinned` / `message_unpinned`).
  A small `● live` indicator flashes on each refresh. Requires the WebSocket
  connected with a filter that includes the thread; UI-only, no backend change.

## [152.0.0] — 2026-08-05

Post-gate hardening (Phase XXIV). Token-efficiency lean reads, part 2 — REST parity (arc item B1). No new gate tag.

### Changed

- **HTTP context pack edits are lean by default** — `GET /threads/:id/context`
  and `GET /workspaces/:wid/context` now serialize `message_edits` as
  `MessageEditView` with **optional** `body_before`/`body_after`; the body
  copies (the largest token cost in a pack) are omitted unless
  **`include_edits=true`** is passed. Brings the REST surface in line with the
  MCP `get_thread_context` default shipped in Cluster 151. The who/when/which
  metadata is always present; the OpenAPI schema registers `MessageEditView`.

### Added

- **`snippet_only=true` on `GET /workspaces/:wid/search`** — drops the full
  message `body` from each hit, returning only the bounded `snippet`. Semantic
  hits (which carry an empty snippet and lean on `body`) get a UTF-8-safe
  truncated body prefix so they still carry locatable content. Default response
  is unchanged.

## [151.0.0] — 2026-08-04

Post-gate hardening (Phase XXIV). Token-efficiency lean reads (arc item B1). No new gate tag.

### Changed

- **`get_thread_context` edits are lean by default** — each edit record now
  carries only `{id, message_id, editor_id, edited_at}` instead of the full
  `body_before` + `body_after` copies, which were the single largest token cost
  in a context pack. New opt-in **`include_edits: true`** restores the full
  before/after bodies. `get_workspace_context` inherits the lean default through
  its nested per-thread packs (its biggest multiplier: N threads × edits). The
  lean record is a strict subset of the full shape, so consumers that ignore
  edit bodies are unaffected.

### Fixed

- **`list_messages` limit is clamped to `1..=500`** — previously unbounded, so a
  negative or very large `limit` could pull the entire thread. Catalog schema
  now advertises the bounds.

## [150.0.0] — 2026-08-04

Post-gate hardening (Phase XXIV). MCP agent surface, part 2 (stream filters). No new gate tag.

### Added

- **`GET /mcp/stream` narrowing by channel / thread / member / kind** — new `channel_id`, `thread_id`, `member_id`, and `kinds` (comma-separated snake_case event kinds; unknown → `400`) query params, wired into the existing `EventFilter`. The WebSocket subscribe already accepted the full filter, but the MCP/SSE stream only wired `workspace_id`/`dm_conversation_id`/`channel_grants` — so an MCP agent had to take the whole workspace firehose and filter client-side. Delivers the "await my mention" primitive: `?workspace_id=…&member_id=…&kinds=mention_recorded`. Completes the MCP-agent-surface pair with 149 (inbox/mentions).

## [149.0.0] — 2026-08-04

Post-gate hardening (Phase XXIV). MCP agent surface, part 1 (inbox/mentions). No new gate tag.

### Added

- **MCP inbox + mention tools** — `list_mentions`, `get_inbox`, `mark_inbox_read` (all `workspace:read`), so an MCP-only agent can discover it was @mentioned. The store + HTTP have had these reads for a while, but they were never in the MCP catalog — an agent could receive a mention (`record_mention` *is* an MCP tool) and have no way to find out. Mirror the HTTP handlers; limits clamp to (1, 500).

## [148.0.0] — 2026-08-04

Post-gate hardening (Phase XXIV). MCP transport spec-completeness, part 4 (final) of the 145–148 arc. No new gate tag.

### Added

- **MCP server→client requests** — the server can now issue JSON-RPC *requests* to a client over its streamable session (`sampling/createMessage`, `roots/list`, `elicitation/create`) via `McpServer::request_client`, gated on the client having declared the matching capability in `initialize` (else `Forbidden`). The request rides the session's SSE stream; the client's response is POSTed back as a JSON-RPC response, which `POST /mcp/streamable` distinguishes from a request (has `id`, no `method`) and routes to the awaiting caller. Closes the "bidirectional" gap — the **MCP streamable spec-completeness arc (145–148) is complete**.
- **Per-session client-capability tracking** — the streamable handler records the client's declared `capabilities` from `initialize` into the session (previously discarded), gating the above.

## [147.0.0] — 2026-08-03

Post-gate hardening (Phase XXIV). MCP transport spec-completeness, part 3 of the 145–148 arc. No new gate tag.

### Added

- **MCP streamable resumability (`Last-Event-ID` replay)** — every session SSE frame now carries a monotonic `id:`; the registry retains the last 256 in a bounded per-session log. `GET /mcp/streamable` with a `Last-Event-ID` header replays the retained frames after that id (as id'd SSE events) before continuing live — the spec's reconnect/redelivery mechanism.

### Changed

- **A streamable session survives a dropped POST stream** (was: the stream's end closed it), so a client can reconnect and replay; TTL/DELETE still clean up. A follow-up POST whose SSE leg has dropped now degrades gracefully to a single JSON response (200) instead of failing — the response is logged for replay regardless.

## [146.0.0] — 2026-08-03

Post-gate hardening (Phase XXIV). MCP transport spec-completeness, part 2 of the 145–148 arc. No new gate tag.

### Added

- **`GET /mcp/streamable`** — the MCP spec's server→client SSE stream on the streamable endpoint. Delivers unsolicited server notifications (e.g. `notifications/resources/updated`) from the server-wide broadcast; touches + echoes an open `Mcp-Session-Id` (`workspace:read`).
- **`Accept`-header content negotiation on `POST /mcp/streamable`** — a client that accepts only `application/json` (no `text/event-stream`) gets a single JSON response instead of an opened SSE session, per the spec's "return SSE or JSON" rule. Absent `Accept` preserves the streaming default.

## [145.0.0] — 2026-08-03

Post-gate hardening (Phase XXIV). MCP transport spec-completeness, part 1 of the 145–148 arc. No new gate tag.

### Added

- **MCP `initialize` protocol-version negotiation** — reads the client's `protocolVersion` and echoes it if supported, else the preferred one (MCP spec §Lifecycle); was: params ignored, version hardcoded. New `maidan-mcp` API: `SUPPORTED_PROTOCOL_VERSIONS` / `is_supported_protocol_version` / `preferred_protocol_version`.
- **`MCP-Protocol-Version` header validation** on `POST /mcp` and `POST /mcp/streamable` — absent is allowed (back-compat), present-but-unsupported → `400`.
- **JSON-RPC batching** on `POST /mcp` — a top-level array is dispatched element-by-element and answered with an array of responses (quota per request); an empty batch → `-32600`.
- **JSON-RPC notifications** (requests without an `id`) are executed for effect and answered `202 Accepted` with no body (single or in a batch); `notifications/initialized` and `notifications/cancelled` are accepted instead of `MethodNotFound`.

## [144.0.0] — 2026-08-03

Post-gate hardening (Phase XXIV). Docs dead-link gate + latent-link cleanup. No new gate tag.

### Added

- **Dead-link gate in the `docs` CI job** — `book.toml` gains an `[output.linkcheck]` renderer (`warning-policy = "error"`, `follow-web-links = false`), so `mdbook build` now fails on a dead internal link instead of shipping it (the class of bug behind the ~20 dead sidebar links fixed in 141). `docs.yml` installs `mdbook-linkcheck`; the second renderer nests HTML under `build/html/`, so the deploy uploads from there.

### Fixed

- **35 latent broken links in the published docs** (surfaced the moment the gate went on): space-named files are now staged under hyphenated names (`Capability Map.md` → `Capability-Map.md`, plus Agent Integration / Open Work / Cluster A) — eliminating `%20`-in-path and giving cleaner URLs (`/maidan/docs/Capability-Map.html`); links out of the published set (unpublished `docs/` pages, repo source, `.github/`, `deny.toml`) are rewritten to absolute GitHub URLs; and `docs/Decisions.md` stray `[`Type`]` bracket-refs (dangling reference-links) were fixed.

### Changed

- **Backlog docs reconciled against shipped code** — `Remaining Work.md §4` no longer lists the global cross-workspace admin-audit API as an open gap (shipped in **132**, UI in **138**); the Slack-parity matrix + Web-UI row now reflect the 134–143 `/ui` track; `Open Work.md`/`Remaining Work.md` baselines bumped to v143.

## [143.0.0] — 2026-06-30

Post-gate hardening (Phase XXIV). UI polish: richer message rendering. No new gate tag.

### Added

- **Timestamps and inline slash-command results in the `/ui` thread view** — `renderMessages` now shows each message's `posted_at` (trimmed) in the meta line, and renders a compact block from `slash_command`/`slash_response` metadata (`⌘ /name args`, ok / error / retrying status, and the handler response). Completes the slash loop: register in the Slash tab (142), run by posting `/name args`, see the result inline. UI-only (no backend); the data was already in the message payload. `ui_js_contract` guard validates the new JS.

### Security

- **Cleared three RustSec advisory-DB findings that had accumulated on the `cargo-deny` gate** (lockfile-only bumps, no `Cargo.toml`/code change):
  - `anyhow` 1.0.102 → 1.0.104 — **RUSTSEC-2026-0190** (unsoundness in `Error::downcast_mut()` on a `.context()`-wrapped error; fixed in `>= 1.0.103`).
  - `crossbeam-epoch` 0.9.18 → 0.9.20 — **RUSTSEC-2026-0204** (invalid pointer dereference in the `fmt::Pointer` impl for `Atomic`/`Shared`; fixed in `>= 0.9.20`).
  - `spin` 0.10.0 → 0.10.1 (and 0.9.8 → 0.9.9) — the 0.10.0 release (via `crc-fast` → `aws-sdk-s3`) was **yanked**.
  - These landed in the RustSec DB over time and failed the required `lint` job's `cargo-deny` advisories check on every PR. `cargo deny check` is clean again locally.

## [142.0.0] — 2026-06-26

Post-gate hardening (Phase XXIV). UI feature: slash-command registry. No new gate tag.

### Added

- **Slash-command registry in the `/ui` console** — a new "Slash" tab to register (name / description / `handler_kind` `http`|`mcp_tool` / `handler_target`), list, and revoke workspace slash commands. For an `http` handler the one-time webhook signing secret is shown once at registration (copy button + warning, like token minting). Backed by new session-gated `/ui/api/workspaces/:wid/slash-commands[/:cid]` routes reusing the tested `slash_commands::*` handlers. Commands still run by posting `/name args` as a message (dispatch is message-triggered; there is no execute endpoint). `ui_js_contract` guard validates the new JS.

## [141.0.0] — 2026-06-26

Post-gate hardening (Phase XXIV). Docs fix: the published site now serves every page. No new gate tag.

### Fixed

- **The published mdBook site shipped a sidebar of ~20 dead links.** mdBook only builds chapter sources under its `src/` dir, but `book/src/SUMMARY.md` referenced the canonical docs with `../docs/...` paths that escape `src/`; mdBook silently skipped them, so only 3 pages (`introduction`, `api`, `mcp-reference`) actually existed and every `docs/*` link 404'd — the links even resolved outside the `/maidan/` base (clicking "Integrating with Maidan" went to GitHub's user-level 404). New `book/sync-docs.sh` stages the 21 SUMMARY-referenced docs into `book/src/docs/` at build time (run by `docs.yml` before `mdbook build`), rewriting out-of-`docs/` repo-root links to absolute GitHub URLs and flattening Obsidian `[[wikilinks]]`. SUMMARY/intro/api links drop the `../`. The site now builds 27 pages (was ~6); the integration guide is reachable from the live nav.

### Added

- **Copy-pasteable local quickstart on the docs landing page** and a **helpful custom 404** (`book/src/404.md`) pointing lost readers to the home + integration guide and noting the `/maidan/` URL prefix.

## [140.0.0] — 2026-06-25

Post-gate hardening (Phase XXIV). UI feature: workspace presence roster. No new gate tag.

### Added

- **Workspace presence roster in the `/ui` console** — a new "Presence" tab showing who's online, rendered from the `presence_snapshot` frames that already ride the existing WebSocket subscribe (the subscribe sends `member_id` when signed in, which registers the operator in the presence hub). Online/Away buttons send `{"type":"presence","status":...}` over the open socket. No backend change — presence is WS-only (no HTTP API). `ui_js_contract` guard validates the new JS.

## [139.0.0] — 2026-06-25

Post-gate hardening (Phase XXIV). UI feature: 1:1 direct messages. No new gate tag.

### Added

- **1:1 direct messages in the `/ui` console** — a new "DMs" tab: open a DM by the other member's ID (the actor is the signed-in member; self-DM rejected), a refreshable list (each row shows the *other* participant), and a conversation pane (select → read, send → post as the actor). Backed by new session-gated `/ui/api` routes — `GET`/`POST` `/ui/api/workspaces/:wid/dm` and `POST /ui/api/dm/:id/messages` — reusing the existing tested `dm::*` handlers; the conversation pane reads through the existing `/ui/api/threads/:tid/messages` (DMs are thread-backed). The exact parallel to group DMs (136). `ui_js_contract` guard validates the new JS.

## [138.0.0] — 2026-06-25

Post-gate hardening (Phase XXIV). UI feature: global audit + reindex controls (completes the operator console). No new gate tag.

### Added

- **Global audit + reindex-embeddings controls in the `/ui` "Operator" tab** — a cross-workspace global-audit view (limit + load; bearer-only, needs `audit:read-global`) and reindex controls: "Reindex this workspace" (`POST {workspace_id}`, `workspace:write`, works on a plain login), "Reindex system-wide" (`POST {}`, `token:admin` bearer), and a poll-by-job-id status readout. Reindex is backed by new session-gated `/ui/api/operator/reindex-embeddings[/:job_id]` routes (the status `GET` lives on the write router because a workspace-scoped job needs `workspace:write` to read) reusing the tested `reindex_ops::*` handlers; global audit calls the top-level `/operator/audit` directly with a bearer. The UI degrades honestly when no token is set. `ui_js_contract` guard validates the new JS.

## [137.0.0] — 2026-06-25

Post-gate hardening (Phase XXIV). UI feature: deliveries & DLQ operator view. No new gate tag.

### Added

- **Deliveries & dead-letter queue in the `/ui` console** — a new "Operator" tab listing webhook + automation deliveries for the current workspace, with a status filter (pending / quarantined / delivered), a kind filter (all / webhook / automation), and a per-row **Replay** to re-attempt a quarantined or failed delivery. Backed by new session-gated `/ui/api` routes — `GET /ui/api/workspaces/:wid/deliveries` (`workspace:read`) + `POST /ui/api/workspaces/:wid/deliveries/:did/replay` (`workspace:write`) — reusing the existing tested `delivery_ops::*` handlers; both map onto the operator-session caps, so the view works on a plain login. Automation auth-header fields are deliberately not rendered. `ui_js_contract` guard validates the new JS.

## [136.0.0] — 2026-06-25

Post-gate hardening (Phase XXIV). UI feature: group DMs. No new gate tag.

### Added

- **Group DMs in the `/ui` console** — a new "Group DMs" tab: open a group DM (comma-separated member ids + optional title; the actor is auto-included and ≥2 members enforced), refresh the list (by member), select a conversation, read its messages, and post as the actor. Backed by new session-gated `/ui/api` routes — `GET`/`POST` `/ui/api/workspaces/:wid/group-dms`, `GET /ui/api/group-dms/:id`, `POST /ui/api/group-dms/:id/messages` — reusing the existing tested `group_dm::*` handlers; the conversation pane reads through the existing `/ui/api/threads/:tid/messages` (group DMs are thread-backed). `ui_js_contract` guard validates the new JS.

## [135.0.0] — 2026-06-25

Post-gate hardening (Phase XXIV). UI feature: message pins. No new gate tag.

### Added

- **Pin/unpin messages in the `/ui` console** — `loadMessages` loads the thread's pins; each message meta shows a 📌 pin/unpin toggle reflecting + flipping state. Backed by new session-gated `/ui/api/threads/:tid/pins` routes (GET/POST/DELETE) reusing the existing tested pin handlers (bearer mode uses the top-level routes). `ui_js_contract` guard validates the new JS.

## [134.0.0] — 2026-06-25

Post-gate hardening (Phase XXIV). UI feature: message reactions. No new gate tag.

### Added

- **Emoji reactions in the `/ui` console** — each message shows aggregated emoji chips with counts (your own highlighted), quick-add buttons (👍 ❤️ ✅ 🎉 👀), and click-to-toggle. Backed by new session-gated `/ui/api/messages/:mid/reactions` routes (GET/POST/DELETE) that mount the existing, tested reaction handlers (bearer mode uses the top-level routes). The `ui_js_contract` guard validates the new JS.

## [133.0.0] — 2026-06-24

Post-gate hardening (Phase XXIV). `/ui` write-path repair + JS guard. No new gate tag.

### Fixed

- **The `/ui` console write path was broken** — its JS called helpers that don't exist: `apiWritePath` / `requireAuthForWrite` (undefined) and `uiApiPath` / `uiWritePath` (typo'd). Create-channel, create-thread, post-message, and attach-artifact threw `ReferenceError`. CI never caught it (no browser; the JS is untested). Defined the two helpers (bearer-or-session) and repointed the typo'd calls (`uiApiPath`→`uiReadPath`, `uiWritePath`→`apiWritePath`).

### Added

- `tests/ui_js_contract.rs` — a dependency-free CI guard (in the `unit tests` job) asserting every bare `ident(` call in `index.html`'s inline script resolves to a definition, a parameter, or a known JS/DOM global. Catches "helper called but never defined" without a browser. (It flagged all four broken references above before the fix.)

## [132.0.0] — 2026-06-24

Post-gate hardening (Phase XXIV). Global admin audit query API. No new gate tag. Completes the 127–132 sweep.

### Added

- **`GET /operator/audit?limit=`** — cross-workspace audit query, gated by a new global capability **`audit:read-global`** (not workspace-scoped; not in `default_minted`). Returns recent-first audit events across all workspaces (`limit` clamped 1..=500). Exposes the existing `Store::list_audit`; the capability is the gate (no org/super-admin model needed). OpenAPI + `http-capability-map.json` wired so the Cluster 121 contract stays green; denial covered by the capability-matrix test, allow by `operator_audit_e2e`.

## [131.0.0] — 2026-06-24

Post-gate hardening (Phase XXIV). Docs-only — delivery-unification verification-close. No new gate tag.

### Changed

- **Closed the "unify webhook + automation delivery" backlog item** as substantially-addressed (verified against code). Signing + backoff are already shared (`automation_delivery` reuses `webhooks::sign_payload`/`delivery_backoff`) and the operator API is unified (`OperatorDelivery`); the two storage tables stay separate **by design** (distinct foreign keys — webhook→subscriptions, automation→slash/fsm). A storage merge was declined as a risky migration with no functional gain; the rationale is recorded in `Remaining Work.md` §3 + `Open Work.md`.

## [130.0.0] — 2026-06-24

Post-gate hardening (Phase XXIV). Test-coverage uplift. No new gate tag.

### Changed

- **observability env-parsing is now unit-tested** via pure extraction: `is_truthy`, `resolve_metrics_endpoint`, `parse_metrics_interval`, `parse_log_format`. The `*_from_env` wrappers feed `std::env::var(...)` into these pure functions, so tests are deterministic and don't mutate process env (which would race the parallel test binary). Behavior is unchanged.
- **maidan-mcp `prompts.rs`** (previously untested) gains a catalog-integrity test.

## [129.0.0] — 2026-06-24

Post-gate hardening (Phase XXIV). Error-visibility + bounded buffers. No new gate tag.

### Fixed

- **Unbounded MCP streamable session buffer** — the per-session SSE channel was `unbounded_channel()`; a slow client could grow server memory without limit. Now a bounded `channel(256)` with non-blocking `try_send` (full buffer logs + disconnects the client; callers already treat a failed push as a gone session).
- **Swallowed outbox quarantine error** (`outbox_relay.rs`) — a failed `quarantine()` was `let _ = …`, leaving the row pending → infinite retry. Now logged (the next tick retries the quarantine).
- **`unreachable!()` in live request handlers** (`delivery_ops` get/replay, `mcp/resources` read) → typed errors, so a future upstream change can't turn a bad input into a process panic.

## [128.0.0] — 2026-06-24

Post-gate hardening (Phase XXIV). A2A delivery robustness. No new gate tag.

### Fixed

- **A2A client could hang indefinitely** — `A2aClient` built a reqwest client with no timeout. Added a 10s `connect_timeout` (all requests) + a 30s per-request timeout on the non-streaming `call`.
- **A2A push notifications were fire-and-forget** — the push POST in `persist_task` swallowed all failures with no retry. Now `deliver_a2a_push` retries 3× with capped exponential backoff, logs each failure, and counts outcomes via `maidan_a2a_push_total{result}`. (Best-effort, not a durable outbox.)
- A2A SSE subscribe poll now logs the `load_task` failure that previously ended the stream silently; the SSE-frame serializer logs on serialize failure instead of emitting a silent empty frame.

## [127.0.0] — 2026-06-24

Post-gate hardening (Phase XXIV). Docs-only — backlog reconciliation. No new gate tag.

### Changed

- **Reconciled `docs/Remaining Work.md` + `docs/Open Work.md` against the code at v126.** Struck ~11 entries listed as open but already shipped (group DMs, presence/typing, per-model embedding tables, `sqlite-vec`, schema-parity tests, cosign signing, bootstrap compile-time strip, SQLite delivery cursor, Helm prod profiles, context thread cursor, Web UI tabs), each with the shipping cluster + evidence. Fixed the stale `Open Work` tail (it still claimed "latest tag v76 / active cluster 78"). Classified the §4 Slack-parity gaps as product/UI (complete backends) vs out-of-scope vs backend-tractable.

## [126.0.0] — 2026-06-24

Post-gate hardening (Phase XXIV). MCP SSE at-least-once parity. No new gate tag.

### Added

- **`at_least_once` on `GET /mcp/stream`** — the Cluster 125 opt-in at-least-once delivery now works on the MCP SSE transport too (query param; requires `workspace_id` + `consumer_id`). Routes the stream through the same `reconcile_deliver` loop the WebSocket path uses (stability-gated, cursor-driven, gap-free, exactly-once per consumer); the optimistic SSE path is unchanged when unset.

### Changed

- `docs/Production.md` — the at-least-once contract now documents both transports (the `/ws/subscribe` frame field and the `/mcp/stream` query param).

## [125.0.0] — 2026-06-23

Post-gate hardening (Phase XXIV). Opt-in at-least-once event delivery. No new gate tag.

### Added

- **Opt-in at-least-once subscriptions.** A `/ws/subscribe` frame with `"at_least_once": true` (requires `filter.workspace_id` + `consumer_id`) switches that subscription to **cursor-driven reconcile** delivery: every committed matching event is delivered in `log_id` order, exactly once per consumer, with **no silent out-of-order gap** (the case the optimistic watermark path can drop on a failed-then-retried outbox row or a late-committing serial). The durable delivery cursor floors re-delivery across reconnects. Default behavior is unchanged for subscriptions that don't opt in.
- **`maidan_events.inserted_at`** (migrations: Postgres 0031, SQLite 0030) — the DB insert wall-clock — plus `Store::list_events_after_stable`, the stability-gated gap-safe read backing the reconcile loop.
- Env: `MAIDAN_DELIVERY_STABILITY_SECS` (default `2`) and `MAIDAN_DELIVERY_RECONCILE_MS` (default `1000`).

### Changed

- `docs/Decisions.md` — new ADR "At-least-once delivery via cursor reconciliation + a time-based stability horizon" (and why dedup was already handled — the real hole was completeness). `docs/Production.md` — the at-least-once subscribe contract (guarantee, latency cost, long-transaction caveat).

## [124.0.0] — 2026-06-23

Post-gate hardening (Phase XXIV). CI / observability loose ends. No new product capability, no new gate tag.

### Removed

- `scripts/validate-prometheus-rules.sh` — a substring-only checker whose `promtool check rules` branch was dead (it ran on the `PrometheusRule` CRD, which promtool can't parse, behind an uninstalled-promtool guard). `scripts/check-alert-rules.sh` (CRD extraction + `promtool check`/`test rules`, the required `promtool (alert rules)` job since v122) is now the sole validator; metric-name presence stays guarded by `alert_templates_contract`.

### Changed

- `scripts/check-alert-rules.sh` now skips gracefully with an install hint when `promtool` is absent (preserving the deleted script's local behavior).
- **Required status checks on `main`: 6 → 8.** `promtool (alert rules)` (v122) and `otlp smoke` (v123) promoted to required branch-protection checks. Docs updated (`CLAUDE.md`, `Operations.md`, `Production.md`, `Capabilities.md`, `docs/alerts/README.md`).

## [123.0.0] — 2026-06-23

Post-gate hardening (Phase XXIV). No new product capability, no new gate tag.

### Added

- **`otlp smoke` CI job + `otlp` compose profile** — end-to-end proof that maidan-server's OTLP export reaches a real OpenTelemetry Collector. `docker/otel-collector-config.yaml` (OTLP/gRPC → debug exporter) + `scripts/otlp-smoke.sh` bring up `postgres` + `otel-collector` + a server with `OTLP_ENDPOINT`/`OTLP_METRICS=1`, drive traffic, and assert the collector received a traces batch (incl. the per-request `http_request` span), a metrics batch, and resource `service.name=maidan-otlp-smoke`. Closes the residual observability gap named by Cluster 122 (the in-process `metrics_push` test never proved delivery to a collector).

### Changed

- `docs/Production.md` — added an OTLP end-to-end verification runbook; the alert-rules validation note now points at the CI-wired `scripts/check-alert-rules.sh` (v122).
- `docs/Remaining Work.md` §1/§3 — OTLP-smoke gap closed (123); corrected the stale "durable job store" line: durable reindex jobs shipped in **Cluster 104** (`maidan_reindex_jobs`).

## [122.0.0] — 2026-06-22

Post-gate hardening (Phase XXIV). No new product capability, no new gate tag.

### Added

- **`promtool (alert rules)` CI job** — executes the SLO recording/alert PromQL on every PR: `promtool check rules` (lint expressions + Go templates) and `promtool test rules` (unit tests). `scripts/check-alert-rules.sh` extracts `.spec` from the `PrometheusRule` CRD into a git-ignored raw rules file first. Closes the "alert exprs are never executed in CI" gap flagged by the Cluster 121 retro (the `alert_templates_contract` test only checks metric *names*).
- **SLO rule unit tests** (`docs/alerts/prometheus-rules-maidan-slo.test.yaml`) pinning the Cluster 121 semantics: `MaidanIndexerQueueSaturated` fires >80% full and is guarded off at capacity 0; `MaidanIndexerEmbedFailures` fires on a rising delta but not on a reset-to-0 (restart-safe).

### Fixed

- **`MaidanIndexerQueueSaturated` annotation** rendered "1000% full": the expr `capacity > 0 and saturation > 0.8` made `$value` the capacity (PromQL `and` returns the LHS). Reordered to `saturation > 0.8 and capacity > 0` so `$value` is the fill fraction ("90% full"); the capacity guard is unchanged. Found by the new promtool unit tests.

### Changed

- **OTLP-export status corrected** (`Remaining Work.md` §1/§3, the [121.0.0] note, and the Cluster 121 plan/retro): OTLP export (traces + metrics fanout) shipped in **Cluster 89** — env-gated, documented in `Production.md` — it was never an open deferral. The genuine residual observability gap is an end-to-end OTLP collector smoke.

## [121.0.0] — 2026-06-22

Post-gate hardening (Phase XXIV) — two named, owner-less backlog gaps closed. No new product capability, no new gate tag.

### Added

- **OpenAPI-wide capability map in CI** (closes the Cluster 69 deferral): `every_openapi_operation_is_bearer_session_or_public` classifies every OpenAPI operation as bearer-mapped (and thus in `contracts/http-capability-map.json`), session-cookie-gated (`/auth/session`, `/auth/session/mint`), or explicitly public (health/metrics/spec/discovery/OIDC handshake). A new route shipping with neither auth nor a capability mapping now fails CI.
- **Scale-out SLO coverage** for the Cluster 116 batched-embed indexer gauges:
  - recording rule `maidan_slo:indexer_queue_saturation` (clamp-guarded queue fill ratio);
  - alert `MaidanIndexerQueueSaturated` — embed queue >80% full (backpressure);
  - alert `MaidanIndexerEmbedFailures` — restart-safe offset-delta on the monotonic `maidan_indexer_embed_failed_total` gauge;
  - operator-dashboard panels for indexer queue depth vs capacity and embed failures;
  - `alert_templates_contract` now asserts the three new indexer metric names.

### Changed

- `docs/Remaining Work.md` §1/§3 — OpenAPI-wide capability map marked closed (121); SLO dashboards/alerts noted as extended to scale-out indexer metrics. (OTLP export was described here as the open observability piece — corrected in [122.0.0]: traces + metrics export shipped in Cluster 89; the open sliver is an end-to-end collector smoke.)

## [120.0.0] — 2026-06-22

### Added

- **`maidan-scale-1.0` product gate** (tagged at this commit), closing Product Ladder 102+:
  - `maidan_scale_gate_e2e` — scale runtime surfaces + indexer lag/queue-depth telemetry respond.
  - `docs/Gates/maidan-scale-1.0.md` — the 7 gate criteria (Clusters 102–119) mapped to test/CI/doc evidence.
  - `crates/maidan-store/benches/STORE_BASELINE.md` — recorded store hot-path bench baseline.
  - `scale-out smoke` CI job promoted to a gate-required check.

## [119.0.0] — 2026-06-22

### Changed

- Workspace moved to **thiserror 2** (source-compatible; our crates on the current major).
- `deny.toml` `[bans] multiple-versions` **warn → deny** — a new duplicate major now fails CI. Unavoidable duplicates are documented exceptions: `skip-tree` for the vendored AWS SDK (`aws-config`/`aws-sdk-s3`) and `openidconnect` v4 subtrees + `testcontainers` (dev), and a `skip` list for cross-cutting ecosystem transitions (getrandom/rand, hashbrown, windows-sys, itertools, metrics-util).

### Added

- `docs/Dependencies.md` — dependency currency + duplicate-version policy, the openidconnect-v5 tracking item (clears base64 0.21 + the rsa advisory RUSTSEC-2023-0071 when released), and the edition-2024 evaluation (compiles; adoption deferred to a Track-V/X migration).

## [118.0.0] — 2026-06-18

### Added

- Hybrid search mode (`mode=hybrid` on HTTP search + the MCP `search_messages` tool): runs lexical and semantic search and fuses their normalized `[0,1]` scores as `combined = w*semantic + (1-w)*lexical`, with `w` = `hybrid_weight` (default 0.5, clamped). Implemented as a `Search::hybrid_search` default trait method (`score::fuse_hybrid`), so both backends inherit it.
- Relevance eval harness (`maidan-search/tests/relevance_eval.rs`): a labeled corpus + controlled synonym embedding asserting hybrid recall dominates both single modes, recovers synonym docs lexical misses, and keeps a top-1-relevant (MRR) floor.

## [117.0.0] — 2026-06-18

### Added

- `Search::ensure_model(provider)` registers the active embedding model's per-model table + registry row at server boot, so a freshly-configured model is queryable before the first write and a dimension mismatch surfaces in startup logs (non-fatal).
- `docs/Embeddings.md` — embedding providers, the per-model table scheme, and the switch-models / reindex workflow.

### Changed

- The `openai-compatible` embedding provider auto-detects its output dimension by probing the endpoint once at boot when `MAIDAN_EMBEDDING_DIM` is unset (instead of defaulting to 1024). A wrong model id or unreachable endpoint now fails at boot with a clear error rather than on every message; set `MAIDAN_EMBEDDING_DIM` explicitly to skip the probe.

## [116.0.0] — 2026-06-17

### Added

- `EmbeddingProvider::embed_batch(bodies)` — default per-item fallback; the OpenAI-compatible provider issues one request with an `input` array (response ordering validated by index + dimension). Backfill (`reindex`) now embeds in chunks of 32 via `embed_batch`.
- Batched live indexing: `BatchingEmbeddingHandler` enqueues live messages onto a **bounded** channel and a worker flushes batches via `embed_batch` (off-runtime). The bounded channel is the backpressure; `queue_depth` is hard-capped by `queue_capacity`, so the indexer-lag metric is bounded. Backfill stays on its own task and never enters the live queue.
- Indexer metrics: `maidan_indexer_queue_depth`, `maidan_indexer_queue_capacity`, `maidan_indexer_embedded_total`, `maidan_indexer_embed_failed_total`, `maidan_indexer_embed_batches_total`. New env: `MAIDAN_INDEXER_QUEUE_CAPACITY` (1024), `MAIDAN_INDEXER_BATCH_SIZE` (32).

## [115.0.0] — 2026-06-17

### Changed

- Non-test `unwrap()`/`expect()` removed from `crates/*/src` (25 sites), each fixed by its nature: lock-poison recovery (`PoisonError::into_inner`), `unreachable!` for infallible constructors (HMAC any-key-length, `EPOCH` constant), `HeaderValue::from_static` for the const problem+json type, `if let` for guarded `pop`/dynamic header parse, `unwrap_or(Value::Null)` for infallible serialize, graceful `tracing::error!` fallback for best-effort metrics init, an explicit `panic!` for the one genuine construction invariant, and `io::Result` + `?` for the codegen bin. A clippy gate (`-D clippy::unwrap_used -D clippy::expect_used` on `--lib --bins`) keeps it at zero; tests may still `unwrap()`.
- `routes.rs` (1617 lines) split into `routes/` domain submodules (workspace, member, channel, thread, message, social, artifact, reference, search, token) and `tools.rs` (1368 lines) into `tools/` (catalog, channel, message, social, artifact, thread, reference, search, automation). Public paths preserved via `mod.rs` re-exports; pure reorganization.

## [114.0.0] — 2026-06-16

### Added

- Round-trip + `proptest` fuzz tests for the JSON-RPC / MCP / A2A envelope surface: `maidan-mcp` request/response/notification shapes and the full `McpError` → JSON-RPC code mapping + `From` conversions; `maidan-a2a` terminal-state classification, `JsonRpcId`, message round-trip / `message_text`, `Task` round-trip, and `maidan_context_from_metadata`.

### Changed

- Coverage gate now measures the **whole test suite** (`cargo llvm-cov nextest --workspace` with a `docker:dind` service) instead of `--lib --bins` only, so the number reflects code exercised by integration tests too (~60% lines vs the old ~16%). `COVERAGE_MIN_LINES` raised **11.0 → 40.0**; coverage-job timeout 45 → 75 min.

## [113.0.0] — 2026-06-15

### Added

- Backend parity guard (`maidan-store/tests/backend_parity.rs`, runs in the required `unit tests` job): asserts every migration *slug* and every `src/{postgres,sqlite}/*.rs` store module exists for both backends, modulo a rationale-documented allowlist (Postgres-only `outbox_quarantine`, folded into `0013_outbox` on SQLite; SQLite-only `pragmas`, no Postgres equivalent). A migration or module added to one backend and forgotten on the other now fails CI. Slug-based (not index-based) because the two migration trees' numbering legitimately diverged.
- Broadened cross-dialect identity test: `run_parity_scenario` / `ParitySnapshot` now also exercise an FSM transition (`Open → InReview`), a message edit (+ edit count), and a reaction, so `dialect_parity` holds both backends to identical results across that wider surface.

## [112.0.0] — 2026-06-15

### Added

- `maidan-fsm` property-test suite (`tests/fsm_properties.rs`, 8 `proptest` properties): `apply` succeeds on exactly the legal `(state, action)` edges (cross-checked against an independent spec table), every legal transition advances the lifecycle rank by exactly one, `Archived` is terminal, rank is monotonic under arbitrary action sequences, the HSM rank ceiling holds for every `(parent, child_to)`, and for an arbitrary rooted thread tree locally-valid edges compose into a tree-wide guarantee (no descendant outruns any ancestor; no internal node archived). Adds `proptest` as a dev-dep. Tests only — no `src/` changes.

## [111.0.0] — 2026-06-15

### Added

- `maidan-auth` integration test suite (26 tests): capability vocabulary + `AuthContext` authorization matrix across token / app-token / session / bypass contexts incl. cross-workspace scoping and constant-time `hashes_equal` edge cases (`capability_matrix.rs`); ChaCha20-Poly1305 peer-secret round-trip, ciphertext/nonce **tamper detection**, truncation/non-base64 rejection, and the `FEDERATION_ENCRYPTION_KEY` parse matrix (`peer_secret_aead.rs`); and store-backed `resolve_bearer` lifecycle — capability propagation, forged-secret rejection, post-revocation and post-expiry failure, plus `resolve_peer_bearer` (`token_lifecycle.rs`). Tests only — no `src/` or production-dependency changes. Opens Phase XXI (correctness & coverage).

## [110.0.0] — 2026-06-12

### Added

- Per-workspace fairness: `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` / `MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS` cap the total request rate for one workspace across all its tokens on `/workspaces/{wid}/…` routes (including search), so a single tenant's heavy loop can't starve others. Independent of the per-client `MAIDAN_RATE_LIMIT_MAX`; both default off and reuse the Redis-optional fixed-window limiter. `tenant_fairness_e2e` proves a capped workspace doesn't degrade another's requests. Documented in `docs/Production.md` (Tenant fairness) and `docs/Threat-Model.md` (T8). Closes Phase XX.

## [109.0.0] — 2026-06-12

### Added

- Configurable pgvector HNSW tuning: `MAIDAN_HNSW_M` and `MAIDAN_HNSW_EF_CONSTRUCTION` set index build params (`CREATE INDEX … WITH (…)`); `MAIDAN_HNSW_EF_SEARCH` sets the per-query candidate list via a transaction-scoped `SET LOCAL hnsw.ef_search`. All optional — defaults are pgvector's own (`m=16`, `ef_construction=64`, `ef_search=40`), preserving current behavior. Build params apply only to indexes created afterward (rebuild via the reindex job to change an existing index). Documented in `docs/Query-Tuning.md`.
- `maidan-search` `criterion` bench (`benches/search_hot.rs`) for lexical (FTS5) and semantic (cosine) latency, with a committed `SEARCH_BASELINE.md` reference for the Cluster 120 perf budgets.

## [108.0.0] — 2026-06-12

### Changed

- Outbox relay cadence is adaptive: it drains pending rows back-to-back (no inter-batch sleep) so a backlog of N rows clears in ≈⌈N/batch⌉ ticks, and backs off its poll interval toward `MAIDAN_OUTBOX_MAX_POLL_INTERVAL_MS` (default 1000 ms) while idle, resetting on the next pending row. A capacity-1 in-process enqueue nudge wakes an idle relay the instant a row is written (polling-safe mpsc; resets the cadence), so the backoff adds no latency to fresh events. At-most-once NOTIFY semantics, metrics, and quarantine are unchanged.

## [107.0.0] — 2026-06-12

### Added

- Database connection pool and timeouts are env-configurable with defaults that reproduce prior behavior: `MAIDAN_DB_MAX_CONNECTIONS` (default: dialect — Postgres 16 / SQLite 8), `MAIDAN_DB_ACQUIRE_TIMEOUT_SECS` (default 30; surfaces a clean error instead of an implicit hang under saturation), `MAIDAN_DB_STATEMENT_TIMEOUT_MS` (Postgres per-connection cap, default 0 = disabled), `MAIDAN_DB_BUSY_TIMEOUT_MS` (SQLite, default 5000). Boot migrations reset `statement_timeout` on their session so a configured cap can't kill the cross-replica advisory-lock wait. Documented in `docs/Production.md` with the `replicas × max_connections` caveat.

## [106.0.0] — 2026-06-12

### Changed

- Context assembly (thread + workspace) now issues a bounded number of store queries independent of message/channel count, eliminating three N+1 patterns. New batched `Store` accessors — `list_threads_for_workspace`, `list_references_from_many`, `list_message_edits_for_messages` (Postgres `= ANY($1)`; SQLite chunked `IN (?, …)`; edits windowed per message) — replace the per-row reads in `thread_context.rs`. Response content and ordering are unchanged. `context_query_count_e2e` guards the bound; `bulk_reads` covers the accessors on both backends.

## [105.0.0] — 2026-06-12

### Added

- Multi-replica scale-out smoke: a `scale` compose profile (two `maidan-server` replicas on one Postgres + a shared object store behind an nginx round-robin LB), `scripts/scale-out-smoke.sh` exercising REST cross-replica paths, and a non-required CI `scale-out smoke` job. `docs/Production.md` documents the supported horizontal-scaling topology (shared vs pod-local state, rolling-update/boot story).

### Fixed

- Boot migrations are serialized across replicas with a Postgres session advisory lock in `run_postgres_migrations`. Concurrent replica starts against a fresh database previously raced on non-transactional DDL (`CREATE EXTENSION` → `pg_extension` unique violation), crashing a replica on startup. `concurrent_migrations` test covers it.

## [104.0.0] — 2026-06-11

### Added

- Durable ephemeral state: OAuth authorization codes and embedding reindex job status now persist in the store instead of per-replica memory, so both work across replicas and survive restart. `maidan_oauth_codes` + `Store::{insert,consume}_oauth_code` (SHA-256 hash only, single-use + TTL enforced atomically via `DELETE … RETURNING`); `maidan_reindex_jobs` + `Store::{upsert,get}_reindex_job` (`ReindexJob`/`ReindexJobStatus` moved to `maidan-types`). `app_oauth.rs` and `reindex_ops.rs` drop their in-memory maps (`AppOAuthRuntime`, `ReindexJobRegistry`) and the `AppState.app_oauth` / `AppState.reindex_jobs` fields. `two_replica_durable_state_e2e` proves a code minted on one replica exchanges on another and a reindex job started on one is observable on another.

### Fixed

- SQLite `apps::parse_ts` now accepts SQLite's `CURRENT_TIMESTAMP` format (naive `YYYY-MM-DD HH:MM:SS`), not just RFC3339 — a latent bug on every SQLite `get_app`, previously masked by Postgres-only app tests.

## [103.0.0] — 2026-06-11

### Added

- Cross-replica presence & roster: `maidan-bus::PresenceNotifier` (`maidan_presence` LISTEN/NOTIFY) so presence, typing, and the workspace roster stay consistent across server replicas. `PresenceHub` keeps a merged, TTL-expiring view with a heartbeat; wired via `AppState::attach_presence_notifier` + `PresenceHub::spawn_tasks` (Postgres NOTIFY mode). `MAIDAN_PRESENCE_HEARTBEAT_SECS` / `MAIDAN_PRESENCE_TTL_SECS` tune it (defaults 10s / 30s). `two_replica_presence_e2e` proves it.

## [102.0.0] — 2026-06-11

### Added

- Cross-replica MCP resource notifications: `maidan-bus::ResourceNotifier` with a Postgres `LISTEN`/`NOTIFY` channel (`maidan_resource_updated`) so `resources/subscribe` SSE updates (`notifications/resources/updated`) reach subscribers on any server replica. Wired via `AppState::attach_resource_notifier` + `McpServer::spawn_resource_notify_listener`; `two_replica_resource_notification_e2e` proves it.

### Changed

- CI: set `RUSTFLAGS=-C debuginfo=line-tables-only` and trimmed the `unit tests` job to `--lib --bins`, stopping recurring `ld` SIGBUS link failures on the runners (and cutting CI time).

## [101.0.0] — 2026-06-03

### Added

- `maidan_operator_gate_e2e` — operator product gate (UI, health, metrics, OpenAPI).

## [100.0.0] — 2026-06-03

### Added

- `maidan mcp-stdio` in-process bus + indexer; `McpServer::with_event_bus` for demo indexing.

## [99.0.0] — 2026-06-03

### Added

- [[Presence and Roster]] documentation; `/ui/api/.../members` roster reads.

## [98.0.0] — 2026-06-03

### Added

- Per-workspace mention webhook route (`GET/PUT /workspaces/:wid/mention-webhook`).

## [97.0.0] — 2026-06-03

### Added

- Multi-member group DM conversations and HTTP API.

## [96.0.0] — 2026-06-03

### Added

- List member API tokens (no secret); `/ui` token list and read-only app installations.

## [95.0.0] — 2026-06-03

### Added

- Faceted search UI aligned with HTTP search API (operator UI v7).

## [94.0.0] — 2026-06-03

### Added

- Artifact cards in `/ui` thread view; upload with optional thread attachment metadata.

## [93.0.0] — 2026-06-03

### Added

- `/ui` WS presets, auto-reconnect, resume tokens; session cookie on `/ws/subscribe`.
- `ui_ws_tail_e2e`.

## [92.0.0] — 2026-06-03

### Added

- `/ui` channel browser: list channels/threads, post messages via session `POST /ui/api/...`.
- `ui_channels_e2e` OIDC session flow without bearer.

## [91.0.0] — 2026-06-03

### Added

- `bootstrap` Cargo feature; production Docker image omits unauthenticated seed routes.
- `bootstrap_absent_e2e` and CI `bootstrap compile-time strip` job.

## [90.0.0] — 2026-06-03

### Added

- SLO alert templates: `docs/alerts/prometheus-rules-maidan-slo.yaml`, Alertmanager route example, validation script.
- Contract test tying alert rules to exported `/metrics` names.

## [89.0.0] — 2026-06-03

### Added

- OTLP metrics push (`OTLP_METRICS`, `OTLP_METRICS_ENDPOINT`) with Prometheus scrape fanout.
- Example Grafana dashboard `docs/dashboards/maidan-operator.json`.
- OpenTelemetry SDK bumped to 0.31 for traces and metrics.

## [88.0.0] — 2026-06-03

### Added

- Helm production profile overlays (OTel, Redis rate limits, S3) and `helm/maidan/PROFILES.md`.
- Helm template smoke coverage for profile combinations.

## [87.0.0] — 2026-06-03

### Added

- Operator reindex job API: `POST /operator/reindex-embeddings`, `GET /operator/reindex-embeddings/:job_id`.
- `Search::reindex_embeddings` for Postgres and SQLite backends.

### Fixed

- SQLite workspace-scoped `maidan reindex-embeddings` / job reindex UUID filter binding.

## [86.0.0] — 2026-06-03

### Added

- Optional `embedding_model` query param on semantic HTTP search and MCP `search_messages`.

## [85.0.0] — 2026-06-02

### Changed

- `sqlite-vec` is an optional Cargo feature on `maidan-search` (default off).
- CI job verifies linkage with `--features sqlite-vec`; SQLite semantic search without the feature uses in-process cosine ranking.

## [84.0.0] — 2026-06-02

### Added

- `MAIDAN_OUTBOX_RELAY_MODE` (`notify` | `polled`) and `MAIDAN_OUTBOX_POLL_INTERVAL_MS`.
- Production guard: `MAIDAN_ENV=production` rejects `MAIDAN_OUTBOX_RELAY=0`.
- SQLite deployments enable outbox relay by default; NOTIFY-loss runbook in [[Production]].

## [83.0.0] — 2026-06-02

### Added

- Product Ladder closure for SQLite `maidan_delivery_cursor` parity (store impl since `v56.0.0`).
- `delivery_cursor` integration tests for Postgres and in-memory SQLite watermarks.

## [82.0.0] — 2026-06-02

### Added

- Context export pagination: `message_cursor` / `thread_cursor` on HTTP and MCP tools.
- `Store::list_messages_after` with stable message ordering (`posted_at`, `id`).

## [81.0.0] — 2026-06-02

### Added

- WS/MCP subscribe `channel_grants` for private channel access control.
- DM subscribe auto-grants the backing private DM channel.

## [80.0.0] — 2026-06-02

### Added

- Unified operator delivery API at `/workspaces/:wid/deliveries` (webhook + automation via `kind`).
- Webhook delivery list/get/replay in store (per workspace).

## [79.0.0] — 2026-06-02

### Added

- A2A `tasks/cancel` RPC and `SubscribeToTask` `statusUpdate` progress frames for non-terminal tasks.
- Terminal subscribe error `-32005`; cancel/progress e2e in `a2a_protocol_e2e`.

## [77.0.0] — 2026-06-02

### Added

- `contracts/http-capability-map.json` and OpenAPI parity CI.
- `http_capability_matrix_e2e` table-driven HTTP capability denial.
- OpenAPI documentation for automation, apps, DMs, workspace context, multipart.

## [76.0.0] — 2026-06-01

### Added

- Agent observability runbook and `agent_substrate_gate_e2e` (`maidan-agent-1.0` gate).

## [75.0.0] — 2026-06-01

### Changed

- Production guidance for real embedding providers and `maidan reindex-embeddings`.

## [74.0.0] — 2026-06-01

### Added

- MCP tools `get_thread_context` and `get_workspace_context`.

## [73.0.0] — 2026-06-01

### Added

- MCP streamable session close e2e; documented session lifecycle in [[Agent Integration]].

## [72.0.0] — 2026-06-01

### Added

- Persisted A2A push config and tasks; `SubscribeToTask` / `tasks/resubscribe` SSE.
- Best-effort HTTP push on task updates.

## [71.0.0] — 2026-06-01

### Added

- `contracts/ws-subscribe-filter.schema.json`; EventKind forward-compat docs.
- MCP resource-notification parity script in CI.

## [70.0.0] — 2026-06-01

### Changed

- [[Architecture]], [[Remaining Work]], [[Open Work]], and root `README.md` reflect **`v69.0.0`** agent substrate (no stale “pins absent” / pre–2.0 stubs).

## [69.0.0] — 2026-06-01

### Added

- `contracts/mcp-capability-map.json` and `contracts/http-capability-routes.json`.
- Table-driven MCP capability matrix e2e (deny + allow gate per tool).
- HTTP capability contract denials in `capability_matrix_e2e`.
- CI: `mcp_capability_map_contract` and `http_capability_map_contract` in `check-agent-contract.sh`.

## [68.0.0] — 2026-06-01

### Added

- Durable signed HTTP delivery queue for slash commands and FSM hooks (`maidan_automation_deliveries`).
- `AutomationDeliveryWorker` with retries, quarantine, and Prometheus metrics.
- Operator API: `GET /workspaces/:wid/automation/deliveries`, `GET .../automation/dlq`, `GET .../deliveries/:did`, `POST .../deliveries/:did/replay`.
- Env: `MAIDAN_AUTOMATION_MAX_ATTEMPTS`, `MAIDAN_AUTOMATION_POLL_INTERVAL_MS`.

## [67.0.0] — 2026-06-01

### Added

- `GET /workspaces/:id/context` packs channels and thread contexts (with message edit history).
- Thread context responses include `message_edits`.

## [66.0.0] — 2026-06-01

### Added

- `/.well-known/maidan.json` documents MCP endpoints and agent card URL.

## [65.0.0] — 2026-06-01

### Added

- App OAuth: `POST .../apps/:app_id/oauth/authorize` and `POST /oauth/app/token` exchange.

## [64.0.0] — 2026-06-01

### Added

- Per-token capability quotas enforced on MCP `tools/call`.

## [63.0.0] — 2026-06-01

### Added

- MCP capability denial covered in `agent_surfaces_e2e`.

## [62.0.0] — 2026-06-01

### Added

- WebSocket `subscribe_ack` includes `schema_version: 1`.
- `GET /workspaces/:wid/outbox/quarantined` lists poison outbox rows.

## [61.0.0] — 2026-06-01

### Added

- `GET /.well-known/agent-card.json` for A2A discovery.
- A2A `tasks/pushNotificationConfig/set` and `/get` for workspace webhooks.

## [60.0.0] — 2026-06-01

### Added

- MCP streamable session TTL (`MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS`, default 3600s).
- `DELETE /mcp/streamable` closes a session (`Mcp-Session-Id` header).

## [59.0.0] — 2026-06-01

### Added

- [[Agent Integration]] guide for external agents.
- Contract golden files: `contracts/event-kinds.json`, `contracts/mcp-tool-names.json`.
- `scripts/check-agent-contract.sh` in CI.

## Maidan 2.0 product gate — 2026-06-01

Tag **[`maidan-2.0`](https://github.com/david-engelmann/maidan/releases/tag/maidan-2.0)**
marks Product Ladder **35–58** completion at the same commit as **`v58.0.0`**.
Checklist: [`docs/Product Completion Checklist.md`](docs/Product%20Completion%20Checklist.md).

Semver **`v2.0.0`** remains **Cluster 2.0** (OIDC identities and human sessions).

## [58.0.0] — 2026-06-01

### Added

- Maidan 2.0 product completion checklist refresh (Clusters 28–57 critical path).
- Expanded `product_completion_gate_e2e`: OpenAPI, metrics, apps, webhooks, app-installations.

## [57.0.0] — 2026-05-31

### Added

- Workspace installed apps: `maidan_apps`, `maidan_app_installations`, bot `MemberKind::Agent` per install.
- App tokens via `api_tokens.app_installation_id`; capabilities must be a subset of the installation grant.
- HTTP: register/list apps, install, list/revoke installations, `POST .../app-installations/:iid/tokens`.

## [56.0.0] — 2026-05-31

### Added

- SQLite `maidan_delivery_cursor` (migration 0023) with real `get` / `advance` store methods.
- `POST /workspaces/:wid/outbox/:outbox_id/replay` clears quarantine for operator recovery (`workspace:write`).

## [55.0.0] — 2026-05-28

### Added

- Helm production bundle: `ingress.annotations`, `values-cert-manager.yaml`, `maidan-stack/values-prod.yaml`.
- `values-ci.yaml` and `scripts/helm-install-kind-smoke.sh` with CI job `helm install (kind)`.
- Helm secrets use `DATABASE_URL` (matches server config).

## [54.0.0] — 2026-05-28

### Added

- Per-token capability quotas: `maidan_token_quotas` and `quotas` on API token mint.
- Quota middleware enforces limits per capability after bearer auth (429 + `Retry-After`).
- Optional Redis rate limiter via `MAIDAN_RATE_LIMIT_REDIS_URL` (global + per-token keys).
- `AuthContext.token_id` for bearer-authenticated requests.

## [53.0.0] — 2026-05-28

### Added

- Workspace full erasure: `DELETE /workspaces/:id` with `confirm_workspace_id` body.
- `Store::erase_workspace` runs deep purge then deletes the workspace row (CASCADE-owned data).

## [52.0.0] — 2026-05-28

### Added

- FSM automation hooks: register handlers for `ThreadStateChanged` transitions (optional `from_state` / `to_state` filters).
- `POST/GET/DELETE /workspaces/:wid/fsm-hooks` with `http` or `mcp_tool` handlers and HMAC signing for HTTP.
- `FsmHookWorker` dispatches on the event bus (covers HTTP transitions and federation-ingested state changes).
- MCP tools `register_fsm_hook` and `list_fsm_hooks`.
- `maidan_fsm_hooks` migrations (Postgres v23, SQLite v21).

## [51.0.0] — 2026-05-29

### Added

- Slash commands: `/name args` parsed on `post_message` when a workspace command is registered.
- `POST/GET/DELETE /workspaces/:wid/slash-commands` with `http` or `mcp_tool` handlers.
- MCP tools `register_slash_command` and `list_slash_commands`.
- Handler results stored on the posted message under `metadata.slash_command` / `metadata.slash_response`.

## [50.0.0] — 2026-05-28

### Added

- Outbound webhooks: subscribe to `EventKind` filters per workspace.
- `POST/GET/DELETE /workspaces/:wid/webhooks` with HMAC-SHA256 signed delivery and retry queue.
- `maidan_webhook_subscriptions` and `maidan_webhook_deliveries` migrations (Postgres v21, SQLite v19).

## [49.0.0] — 2026-05-28

### Added

- `GET /threads/:id/context` — messages, references, metadata-linked artifacts, FSM history.
- `Store::list_thread_transitions` for thread lifecycle audit in context export.

## [48.0.0] — 2026-05-29

### Added

- `sqlite-vec` loaded per SQLite connection; SQL-side `vec_distance_cosine` for semantic search.
- `SearchHit.score` in `[0, 1]` — comparable across Postgres and SQLite within one search mode.

## [47.0.0] — 2026-05-29

### Added

- Per-model embedding tables (`maidan_embedding_models`, `maidan_emb_*`) for mixed dimensions.
- `maidan reindex-embeddings` CLI to rebuild vectors after provider change.

## [46.0.0] — 2026-05-29

### Added

- `maidan_message_edits` stores body before/after on each edit.
- `GET /messages/:id/edits` and `GET /ui/api/messages/:mid/edits`.
- UI v5: “edited” labels and edit history panel in the collab view.

## [45.0.0] — 2026-05-29

### Added

- UI v4 admin tab: workspace audit log, purge confirmation, federation peer admin.
- Token mint with capabilities and revoke by ID in `/ui`.
- `GET /ui/api/workspaces/:wid/audit` and `GET /ui/api/workspaces/:wid/peers`.

## [44.0.0] — 2026-05-29

### Added

- UI v3 collaboration at `/ui`: thread list, post/edit messages, artifact upload, faceted search.
- Session/bearer read proxies: `GET /ui/api/channels/:cid/threads`,
  `GET /ui/api/threads/:tid/messages`, `GET /ui/api/workspaces/:wid/search`.

## [43.0.0] — 2026-05-29

### Added

- UI v2 at `/ui`: responsive shell, workspace channel list, WebSocket live event tail.
- `GET /ui/api/workspaces/:wid/channels` for browser session or bearer.

## [42.0.0] — 2026-05-29

### Added

- WebSocket ephemeral presence (`presence_snapshot`, online/away/offline) and typing
  indicators when subscribe includes `member_id` and `filter.workspace_id`.

## [41.0.0] — 2026-05-29

### Added

- Emoji reactions: `maidan_reactions`, message reaction HTTP API, MCP tools, and bus events.
- Thread pins: `maidan_pins`, pin/unpin/list HTTP API, MCP tools, and bus events.

## [40.0.0] — 2026-05-29

### Added

- Member inbox: `maidan_inbox_cursor`, `GET /members/:id/inbox`, `POST /members/:id/inbox/read`.
- Baseline `@handle` mention routing in `maidan-router` when messages are posted (HTTP + MCP).

## [39.0.0] — 2026-05-29

### Added

- Direct messages: `maidan_dm_conversations` schema, HTTP `POST/GET /workspaces/:id/dm`,
  `POST/GET /dm/:id/messages`, MCP `open_dm_conversation` / `list_dm_conversations` /
  `post_dm_message`, and WebSocket `filter.dm_conversation_id` (resolves to thread).

## [38.0.0] — 2026-05-29

### Added

- MCP `notifications/resources/updated` fan-out on HTTP `edit_message`, `purge_workspace`,
  `create_mention`, and `cast_vote`.

## [37.0.0] — 2026-05-29

### Added

- A2A `SendStreamingMessage` on `POST /a2a/v1/rpc`: SSE stream of JSON-RPC frames with initial
  `Task` and `TaskStatusUpdateEvent` when a message is posted.

## [36.0.0] — 2026-05-29

### Added

- `maidan mcp-stdio` supports Postgres `DATABASE_URL` (`PostgresStore` + `PostgresSearch`).

## [35.0.0] — 2026-05-29

### Added

- MCP streamable HTTP bidirectional mux: follow-up `POST /mcp/streamable` requests with an open
  `Mcp-Session-Id` return JSON-RPC responses pushed to the original SSE session.

## [34.0.0] — 2026-05-29

### Added

- `Mcp-Session-Id` response header on `POST /mcp/streamable` for streamable HTTP session correlation.

## [33.0.0] — 2026-05-29

### Added

- MCP `notifications/resources/updated` fan-out when HTTP tombstones a message or transitions thread FSM state.

## [32.0.0] — 2026-05-29

### Added

- `helm/maidan-stack` umbrella chart with optional Bitnami PostgreSQL and MinIO dependencies.
- Helm template smoke covers maidan-stack when `Chart.lock` is present.

## [31.0.0] — 2026-05-28

### Added

- Workspace deep purge removes artifact metadata for workspace members and deletes content-addressed blobs from the artifact store.
- `WorkspacePurgeResult.artifacts_removed`; audit metadata `artifact_blobs_deleted`.

## [30.0.0] — 2026-05-28

### Added

- Optional HTTP rate limiting via `MAIDAN_RATE_LIMIT_MAX` and `MAIDAN_RATE_LIMIT_WINDOW_SECS`.
- `429 Too Many Requests` with `application/problem+json` and `Retry-After`.

## [29.0.0] — 2026-05-28

### Added

- `PATCH /messages/:id` — edit message body/metadata; sets `edited_at`; publishes `MessageEdited`.
- MCP `edit_message` tool with author vs moderator capability rules.
- Search indexer and embedding handler react to `MessageEdited`.

## [28.0.0] — 2026-05-28

### Added

- Deep workspace purge: embeddings, references, API token revocation, event log removal; extended `WorkspacePurgeResult` counts.
- `GET /workspaces/:id/audit` — workspace-scoped audit trail for operators.

### Changed

- `POST /workspaces/:id/purge` audit metadata includes full purge counts.

## [27.0.0] — 2026-05-28

Major release: **Product Ladder 17–27 close** (clusters 23–27 shipped in PR #198;
CHANGELOG sections v23–v26 record logical cluster boundaries at the same merge).

### Added

- MCP streamable HTTP: `POST /mcp/streamable` returns JSON-RPC response then SSE notifications on one connection.
- Post-ladder backlog: `docs/Remaining Work.md` and vault refresh.

### Documentation

- Retros: `docs/Retros/Cluster 23.0.md` … `Cluster 27.0.md`.

## [26.0.0] — 2026-05-28

### Added

- Product completion checklist and `product_completion_gate_e2e` smoke.

## [25.0.0] — 2026-05-28

### Added

- `POST /workspaces/:id/purge` workspace message erasure with `workspace.purge` audit events.

## [24.0.0] — 2026-05-28

### Added

- `helm/maidan` chart (Deployment, Service, ConfigMap, Secret, Ingress, HPA, PVC) and `scripts/helm-template-smoke.sh`.

## [23.0.0] — 2026-05-28

### Added

- Web UI tabs: events, search, thread FSM transitions, member API token mint.

## [22.0.0] — 2026-05-28

### Added

- Capability map documentation and denial e2e tests for HTTP, MCP, A2A, and WS.

## [21.0.0] — 2026-05-28

Major release: Google A2A protocol v1.0 ingress and client.

### Added

- `POST /a2a/v1/rpc` with `SendMessage` and `GetTask`.
- `maidan-a2a::A2aClient` and protocol types.

## [20.0.0] — 2026-05-28

Major release: message router crate wired into HTTP and MCP.

### Added

- `maidan-router` resolve helpers for channel, thread, and message chains.
- SQLite integration test; server and MCP fan-out use the router.

## [19.0.0] — 2026-05-28

Major release: S3 multipart large artifacts.

### Added

- S3 multipart upload API and MinIO integration test.
- HTTP multipart routes and MCP multipart tools.

## [18.0.0] — 2026-05-28

Major release: SQLite semantic search.

### Added

- SQLite `maidan_message_embeddings` migration and semantic `Search` impl.
- HTTP/MCP `mode=semantic` on SQLite backends.

### Changed

- Cosine ranking in Rust (sqlite-vec SQL deferred; see Decisions).

## [17.0.0] — 2026-05-28

Major release: MCP resource fan-out for tool mutations.

### Added

- `maidan-mcp::resource_updates` resolves thread, channel, workspace, and artifact URIs from mutating tools.
- Notifications fan out to all subscribed related resources.

### Changed

- MCP reference documents multi-URI fan-out behavior.

## [16.0.0] — 2026-05-28

Major release: MCP HTTP resource notification SSE.

### Added

- Shared `McpServer` on `AppState` for persistent HTTP subscriptions.
- `GET /mcp/notifications` SSE stream of JSON-RPC notifications.
- Broadcast fan-out for `notifications/resources/updated` (HTTP + stdio).

### Changed

- `POST /mcp` uses shared dispatcher; MCP reference documents HTTP notifications.

## [15.0.0] — 2026-05-28

Major release: MCP resource subscriptions (stdio first).

### Added

- MCP JSON-RPC methods `resources/subscribe` and `resources/unsubscribe`.
- Stdio notification delivery: `notifications/resources/updated`.
- Resource URI validation helper in `maidan-mcp`.
- `post_message` trigger mapping to notify subscribed `maidan://threads/{id}` resources.

### Changed

- MCP reference now documents subscription methods and notification shape.

## [14.0.0] — 2026-05-28

Major release: SQLite transactional outbox parity.

### Added

- SQLite `maidan_outbox` migration and transactional `append_event`.
- `OutboxBackend` for Postgres and SQLite; relay + metrics on both dialects.
- SQLite deployments run outbox relay against `InMemoryBus` after commit.

### Changed

- `AppState.outbox_backend` replaces `outbox_pool` for dialect-neutral metrics.

## [13.0.0] — 2026-05-27

Major release: delivery cursors and subscriber idempotency contract.

### Added

- Postgres `maidan_delivery_cursor` (`consumer_id`, `workspace_id` → `last_delivered_log_id`).
- Optional `consumer_id` on WebSocket subscribe and MCP SSE; replay floors from stored cursor.
- Federation ingest advances `federation:{peer_id}` cursor after successful handoff.
- Delivery contract documented in Decisions, Architecture, Production.

## [12.0.0] — 2026-05-27

Major release: outbox relay quarantine and operator metrics.

### Added

- `quarantined_at` on `maidan_outbox`; relay skips quarantined rows.
- `MAIDAN_OUTBOX_MAX_ATTEMPTS` (default 16) caps failed relay retries.
- Metrics `maidan_outbox_quarantined`, `maidan_outbox_oldest_pending_seconds`,
  `maidan_outbox_relay_total{result="quarantined"}`.
- Production runbook for quarantine triage and manual recovery.

## [11.0.0] — 2026-05-27

Major release: coverage depth — outbox/relay tests and CI floor at 11%.

### Added

- Postgres outbox integration tests (`record_attempt`, `mark_published`, ordering).
- `maidan-bus::test_support` bus doubles (`FailingBus`, `RecordingBus`).
- Server tests: `publish` deferral when `outbox_relay`, relay failure path, HTTP outbox e2e,
  `/metrics` outbox gauges, `GET /ui/` static e2e.

### Changed

- `COVERAGE_MIN_LINES` raised from **10.5** to **11.0** in CI.

## [10.0.0] — 2026-05-27

Major release: Postgres transactional outbox for commit-then-publish ordering.

### Added

- `maidan_outbox` table; `append_event` enqueues outbox rows in the same transaction.
- `OutboxRelay` background task publishes pending rows via `PostgresBus`.
- Metrics `maidan_outbox_pending` and `maidan_outbox_relay_total{result}`.
- Integration tests for outbox enqueue and relay delivery.

### Changed

- Postgres `publish()` defers direct `bus.publish` to the relay; SQLite unchanged.
- Federation ingest uses a single `publish()` path (fixes double append).

## [9.0.0] — 2026-05-27

Major release: coverage depth — targeted tests and raised CI line floor.

### Added

- Unit/e2e tests for `EventFilter`, bus hydrate/error paths, subscribe metrics,
  `/metrics` hydrate scrape, search query edges, and auth peer decrypt failure.

### Changed

- `COVERAGE_MIN_LINES` raised from **10.0** to **10.5** in CI.
- WS auto-replay integration test timeout extended for slow CI hosts.

## [8.0.0] — 2026-05-27

Major release: Postgres bus hydrate observability on `/metrics`.

### Added

- `maidan_bus_notify_hydrate_total{result}` (`ok`, `not_found`, `failed`,
  `invalid_payload`) for Postgres NOTIFY pointer hydrations.
- `HydrateStats` in `maidan-bus`; exported via `AppState.bus_hydrate_stats` on scrape.
- Production/Operations/Architecture hydrate alerting and troubleshooting.

### Changed

- OpenAPI `/metrics` description includes hydrate series (Postgres deployments).

## [7.0.0] — 2026-05-27

Major release: Postgres bus pointer delivery — NOTIFY carries `log_id`, listener
hydrates from `maidan_events`.

### Added

- `Store::get_stored_event(log_id)` on Postgres and SQLite.
- Postgres `NOTIFY` pointer payload (`log_id_v1`) with listener hydration;
  `BusError::HydrateNotFound` and `HydrateFailed` for missing or corrupt rows.
- Integration tests for pointer round-trip and large persisted events.

### Changed

- Postgres `publish` with `log_id > 0` no longer ships full envelopes on NOTIFY
  (legacy full JSON retained for `log_id == 0` synthetic publishes).
- [[Architecture]], [[Decisions]], and [[Production]] document pointer-default
  semantics and unchanged at-most-once standing risk.

## [6.0.0] — 2026-05-27

Major release: delivery reliability observability for subscribe recovery and
background task health.

### Added

- Prometheus metrics for subscriber lag/recovery across WebSocket and MCP SSE:
  `maidan_bus_lag_total`, `maidan_bus_lag_skipped`, and
  `maidan_subscribe_replay_total{transport,outcome}`.
- Runtime gauges on `/metrics`: `maidan_indexer_last_event_age_seconds`,
  `maidan_bus_listener_ok`, and `maidan_bus_listener_errors_total`.
- Production/Operations/Architecture guidance for delivery reliability alerts and
  troubleshooting.

### Changed

- Full `compose.yaml` profile now sets `INDEXER_STALE_SECS=300` to surface indexer
  silence in readiness during smoke-style runs.

## [5.0.0] — 2026-05-27

Major release: coverage uplift, optional Codecov, and model-aware semantic search.

### Added

- Targeted unit tests; CI line-coverage floor raised to **10.0%** (`COVERAGE_MIN_LINES`).
- Optional Codecov upload from the `llvm-cov` job when `CODECOV_TOKEN` is configured.
- Postgres `semantic_search` filters embeddings by the active provider `model`.
- `SearchHit.embedding_model` on semantic hits; `/health` reports embedding model and dimension when enabled.
- Architecture and Production documentation for lexical vs semantic `rank` semantics.

### Changed

- `Search::semantic_search` takes an explicit `model` argument (breaking for implementors).
- OpenAPI `SearchHit` schema includes optional `embedding_model`.

## [4.0.0] — 2026-05-27

Major release: subscriber continuity with signed resume tokens and replay truncation signaling.

### Added

- HMAC-signed `resume_token` and `subscribe_ack` on WebSocket subscribe and MCP SSE (`/mcp/stream`).
- `replay_truncated` control frame when event-log replay returns 500 rows (`REPLAY_LIMIT`).
- Production and Architecture documentation for subscribe/resume; OpenAPI `info.description` summary.
- E2e: resume-token reconnect and `replay_truncated` when the log exceeds one replay window.

### Changed

- Full-profile `compose.yaml` sets `MAIDAN_SESSION_SECRET` so auth-on smoke tests start with resume signing configured.

## [3.0.0] — 2026-05-27

Major release: search/subscriber depth with semantic facets, automatic lag replay, and a CI coverage floor.

### Added

- Semantic facets on Postgres search (`author`, `channel`, `kind`) for `mode=semantic` on HTTP and MCP.
- Automatic WS/MCP replay from `maidan_events` when subscribers lag and `workspace_id` scope is present.
- Coverage gate in CI with `cargo llvm-cov --fail-under-lines` (`COVERAGE_MIN_LINES=9.0`).

### Changed

- `replay_hint` is now a fallback path (no workspace filter or replay failure), not the primary lag path when workspace scope exists.
- Operations runbook documents the measured baseline (9.8% lines from run `26485125992`) and gate bump policy.

## [2.1.0] — 2026-05-26

Minor release: OIDC operator hardening after `v2.0.0`.

### Added

- HMAC-signed `maidan_session` cookies; unsigned bare UUID cookies rejected.
- IdP `end_session_endpoint` discovery and redirect on `POST /auth/logout`.
- OpenAPI documentation for auth/session routes and `sessionCookie` security scheme.
- `MAIDAN_OIDC_AUTO_MINT=1` redirects to `/ui/?auto_mint=1` when no `token:admin` exists.
- `/ui/` improvements: session-aware controls, one-time secret banner, copy-to-clipboard.

### Changed

- `MAIDAN_SESSION_SECRET` is load-bearing for cookie integrity (invalidates existing sessions on upgrade).
- OpenAPI document version `2.1.0`.

## [2.0.0] — 2026-05-26

Major release: runtime OIDC human login, server-side sessions, and browser UI
integration. Agent MCP/A2A paths remain bearer-token authenticated.

### Added

- Migration `0012`: `maidan_oidc_identities`, `maidan_sessions`, `maidan_oidc_pending`.
- OIDC routes: `GET /auth/oidc/login`, `GET /auth/oidc/callback`, `POST /auth/logout`.
- Session routes: `GET /auth/session`, `POST /auth/session/mint` (first `token:admin` per workspace).
- `GET /ui/api/workspaces/:wid/events` with session-or-bearer middleware.
- `/ui/` HTML: OIDC sign-in, session status, first-admin token mint, cookie-backed events.
- `MAIDAN_OIDC_*` and `MAIDAN_SESSION_*` configuration (see `docs/Production.md`).
- `Store::workspace_has_active_capability` for admin-mint gating.
- `openidconnect` v4 client with mock IdP for tests (`MAIDAN_OIDC_MOCK=1`).

### Changed

- `docs/OIDC.md` design spike superseded by runtime implementation.
- `deny.toml`: ignore `RUSTSEC-2023-0071` for transitive `rsa` via `openidconnect`.

## [1.4.0] — 2026-05-26

Auth hardening minor: bootstrap route gating and OIDC design planning.

### Added

- `MAIDAN_BOOTSTRAP=1` gate on unauthenticated bootstrap routes when auth is enabled.
- One-shot bootstrap workspace seed behavior (`POST /workspaces` returns 403 after first workspace).
- OIDC human login design spike document (`docs/OIDC.md`) with phased `v2.0.0` plan.

### Changed

- `Store` gained `count_workspaces` for bootstrap enforcement.
- Production and threat-model docs now reflect bootstrap gating and OIDC deferral.

## [1.3.0] — 2026-05-26

Semantic search UX minor: HTTP/MCP semantic mode, remote embedding provider
support, and readiness visibility for embedding/indexer failures.

### Added

- `mode=semantic` for `GET /workspaces/:wid/search` (Postgres semantic ranking).
- MCP `search_messages.mode` (`lexical` / `semantic`) with parity behavior.
- OpenAI-compatible embedding provider via env:
  `MAIDAN_EMBEDDING_PROVIDER=openai-compatible`,
  `MAIDAN_EMBEDDING_ENDPOINT`, `MAIDAN_EMBEDDING_MODEL`,
  optional `MAIDAN_EMBEDDING_API_KEY`, `MAIDAN_EMBEDDING_DIM`,
  `MAIDAN_EMBEDDING_TIMEOUT_SECS`.
- `/health/ready` now reports embedding indexer errors.

### Changed

- Semantic query paths now fail fast on embedding provider errors (HTTP + MCP).
- `EmbeddingProvider::embed` returns `Result<Vec<f32>, EmbeddingProviderError>`.

## [1.2.0] — 2026-05-26

Search + embeddings minor: pluggable provider hook, faceted lexical search,
Postgres web-style query operators.

### Added

- `EmbeddingProvider` trait and `MAIDAN_EMBEDDING_PROVIDER` (default `hash-v1`).
- Optional `author`, `channel`, and `kind` filters on workspace search (HTTP + MCP).
- Postgres `websearch_to_tsquery` when `q` contains quotes, `-negation`, or `or`.

### Changed

- `Search::search_messages` accepts `SearchFilters`; both backends apply facets in SQL.

## [1.1.0] — 2026-05-24

Delivery reliability minor: bus health, client replay, federation secrets + pull smoke.

### Added

- Postgres `LISTEN` task health on `/health/ready` (`bus` field).
- WebSocket and MCP `replay_hint` when the in-process bus subscriber lags.
- `after_id` on `/ws/subscribe` and MCP stream; persisted event replay on connect.
- Migration 0010: ChaCha20-Poly1305 encrypted peer outbound bearer secrets (`FEDERATION_ENCRYPTION_KEY`).
- Migration 0011: `maidan_peers.remote_workspace_id` for cross-instance poll.
- `scripts/federation-pull-smoke.sh` and CI pull-path compose coverage.

### Changed

- Federation poll worker resolves outbound secrets from DB after restart.
- `CreatePeer` accepts optional `remote_workspace_id`.

## [1.0.0] — 2026-05-24

Production gates and semver-stable public API. Deployment guidance in
`docs/Production.md`. Liveness/readiness probes and production config
guards shipped in `v0.7.0`; this release documents the contract and
freezes breaking-change policy.

### Added

- `docs/Production.md` production runbook.
- Documented API stability policy (see `docs/Decisions.md`).

## [0.7.0] — 2026-05-24

End of Cluster H. Web UI, MCP stdio, SSE stream, production ergonomics.

### Added

- Graceful shutdown and `X-Request-Id` middleware.
- `/health/live` and `/health/ready` probes.
- `maidan mcp-stdio` CLI transport.
- `GET /mcp/stream` SSE for subscribed events.
- Minimal browser UI at `/ui/`.
- `docs/Production.md`; `MAIDAN_ENV=production` forbids `AUTH_DISABLED`.

## [0.6.0] — 2026-05-24

End of Cluster G. Maidan-native federation between deployments.

### Added

- Migration 0009 `maidan_peers` and `maidan_federated_ingest` dedupe table.
- `maidan-a2a` federation envelope, batch validation, and `Outbound` HTTP client.
- `POST /a2a/v1/events` inbound ingest with peer bearer auth.
- `FederationWorker` background poll (`FEDERATION_POLL_INTERVAL_SECS`, `FEDERATION_DISABLED`).
- Peer admin API and `GET /.well-known/maidan.json` agent card.
- Capabilities `federation:ingest` and `federation:admin`.

## [0.5.0] — 2026-05-23

End of Cluster F. API tokens, capabilities, and auth on HTTP, WebSocket, and MCP.

### Added

- Migration 0008 `maidan_api_tokens` + store CRUD (create, lookup, revoke).
- `maidan-auth` — token hashing, capability vocabulary, `AuthContext`.
- HTTP Bearer middleware; `AUTH_DISABLED=1` for tests and bootstrap.
- Per-route capability checks with RFC 7807 401/403 responses.
- WebSocket `SubscribeFrame.token` with `event:subscribe` enforcement.
- MCP auth on `tools/call`, `resources/read`, `prompts/get`.
- `POST /workspaces/:wid/members/:mid/tokens` and `DELETE /tokens/:id`.

## [0.4.0] — 2026-05-23

End of Cluster E. Artifacts are first-class: S3-backed object storage,
typed kinds, HTTP upload/download, and MCP tools.

### Added

- `ArtifactKind` (`screenshot`, `recording`, `transcript`, `code_dump`, `attachment`).
- Migration 0007 kind CHECK on both dialects.
- `S3Store` with MinIO testcontainers integration test.
- `POST /artifacts`, `GET /artifacts/:sha`, `GET /artifacts/:sha/meta`.
- `put_reader` streaming helper and kind-aware `put_*` helpers.
- MCP `upload_artifact`, `get_artifact_metadata`, `maidan://artifacts/{sha}`.

### Changed

- Compose `full` profile uses `ARTIFACT_BACKEND=s3` + `minio-init` bucket job.
- Rust toolchain pinned to **1.91** (AWS SDK MSRV).

## [0.3.0] — 2026-05-23

End of Cluster D. Thread lifecycle is FSM-driven with a persistent
transition log, hierarchical nested threads, Postgres embedding
indexing, event replay, and MCP workflow prompts.

### Added

- `maidan-fsm` thread lifecycle (`open` → `in_review` → `closed` → `archived`).
- Schema 0004 `maidan_thread_transitions`; schema 0005 `parent_thread_id`.
- `POST /threads/:id` with `start_review`, `close`, `archive` actions.
- `ThreadStateChanged` on the event bus.
- `maidan_fsm::hsm` parent/child state ordering for nested threads.
- `EmbeddingHandler` with `hash-v1` deterministic 1024-d vectors (Postgres).
- Schema 0006 `maidan_events` persistent log + `GET /workspaces/:wid/events`.
- MCP `prompts/list` and `prompts/get` (`thread_workflow`).

### Changed

- `ThreadState` includes `in_review`.
- Server publishes append to `maidan_events` before bus notify.

## [0.2.0] — 2026-05-23

End of Cluster C. The workspace is now searchable: lexical search on
both backends, vector search on Postgres, and the async indexer
pipeline that future clusters will use for embedding generation.

### Added

- `maidan-search::Search` async trait with `search_messages`,
  `upsert_embedding`, `semantic_search`.
- `PostgresSearch` lexical impl using `tsvector` + GIN index +
  `ts_headline` snippets (migration 0002).
- `SqliteSearch` lexical impl using FTS5 + `snippet()` (migration
  0002). FTS5 grammar-escaped queries.
- `PostgresSearch` semantic impl using `pgvector` `vector(1024)` +
  HNSW cosine index (migration 0003). SQLite returns
  `SearchError::Unsupported` for semantic methods.
- `GET /workspaces/:wid/search?q=...&limit=...` HTTP route with
  RFC 7807 `application/problem+json` errors on bad input.
- MCP `search_messages` tool (8th tool overall) sharing the same
  `Arc<dyn Search>` as the HTTP route.
- `maidan-search::Indexer` task: subscribes to the bus, reconnects
  with exponential backoff, dispatches to a swappable `EventHandler`.
- `LoggingHandler` baseline + `wait_for(timeout, predicate)` test
  helper.
- `maidan-server::main` wires the indexer on boot and shuts it
  down cleanly on serve exit.

### Changed

- Every Postgres testcontainer in the workspace switched from
  `postgres:17-alpine` to `pgvector/pgvector:pg17` so migration
  0003's `CREATE EXTENSION vector` succeeds.
- `AppState::new` signature gained `search: Arc<dyn Search>`.
- `McpServer::new` signature gained the same.

### Security

- FTS5 query strings are escaped before binding to prevent grammar
  injection. (Not a SQL injection concern — values are always
  parameterized — only an FTS5 operator concern.)

## [0.1.0] — 2026-05-23

End of Cluster B. The substrate from `v0.0.1` is now reachable: HTTP
CRUD covers the core entity set, every mutation publishes to the bus,
clients can subscribe over WebSocket, and an MCP surface exposes the
workspace as tools and resources to agents.

### Added

- GitHub Actions CI workflows: `lint` (fmt + clippy + cargo-deny),
  `secrets` (trufflehog), `test` (unit), `integration`
  (testcontainers Postgres + in-memory SQLite), `e2e` (docker compose
  + `/health` smoke). All five required-status-checks on `main`.
- Nightly mutation + benchmark workflow skeleton (informational).
- Release workflow that builds cross-arch binaries (Linux x64/arm64
  + macOS x64/arm64) and multi-arch ghcr.io images on `v*.*.*` tag
  push.
- HTTP CRUD routes for workspaces, members, channels, threads,
  messages (incl. tombstone via `DELETE`), mentions, votes,
  references. RFC 7807 `application/problem+json` errors via a
  custom `ApiJson` extractor.
- Event taxonomy in `maidan-types`: `Event` enum
  (`WorkspaceCreated`, `MemberJoined`, `ChannelCreated`,
  `ThreadCreated`, `MessagePosted`, `MessageTombstoned`,
  `MentionRecorded`, `VoteCast`, `ReferenceAdded`,
  `ArtifactUpserted`), `EventKind`, `EventFilter`.
- `maidan-bus::EventBus` async trait, `InMemoryBus` (tokio
  broadcast), `PostgresBus` (`LISTEN`/`NOTIFY` with a 7990-byte
  payload cap and `BusError::PayloadTooLarge`).
- Every HTTP mutation publishes the corresponding event after the
  store commit succeeds; publish errors are logged but do not turn
  successful mutations into 5xx.
- `GET /ws/subscribe` WebSocket endpoint with filter handshake,
  30 s ping / 60 s pong-timeout, bounded 256-cap mpsc backpressure,
  documented close codes (1000, 1002, 1008, 1011).
- `maidan-mcp` crate: transport-agnostic JSON-RPC 2.0 dispatcher
  supporting `initialize`, `tools/list`, `tools/call`,
  `resources/list`, `resources/read`.
- 7 MCP tools (`list_channels`, `list_threads`, `list_messages`,
  `post_message`, `record_mention`, `cast_vote`, `add_reference`)
  with typed input schemas.
- 3 MCP resource URI patterns (`maidan://workspaces/{id}`,
  `maidan://channels/{id}`, `maidan://threads/{id}`).
- `POST /mcp` HTTP endpoint wraps the MCP dispatcher.
- Integration tests: HTTP CRUD on both backends, event emission
  end-to-end, WS subscription with filters + bad-handshake close,
  MCP full flow + parse error.

### Changed

- `AppState::new` signature gained an `Arc<dyn EventBus>` parameter.
- `axum` now uses the `ws` feature.
- `docker/Dockerfile.db` no longer bundles schema into
  `docker-entrypoint-initdb.d` — the server's migration runner is
  the single source of truth.
- `deny.toml`: `allow-wildcard-paths = true` to permit workspace
  path deps; transitive testcontainers advisories
  (`RUSTSEC-2025-0134`, `RUSTSEC-2025-0111`) explicitly ignored
  with rationale.
- Every workspace crate now sets `publish.workspace = true` and
  the workspace inherits `publish = false`.

### Security

- `trufflehog --only-verified` runs on every PR in CI.
- `cargo-deny` runs on every PR in CI.
- Branch protection on `main` now requires the 5 CI jobs to pass.

## [0.0.1] — 2026-05-22

First tagged release. End of Cluster A. The repo is now a working
substrate: it builds, tests, deploys via Docker and Kubernetes, and
exposes a `/health` endpoint backed by Postgres or SQLite.

### Added

- MIT LICENSE, CONTRIBUTING.md, SECURITY.md, CHANGELOG.md,
  `.gitignore`, `.editorconfig`, `rust-toolchain.toml` (pinned to 1.88).
- Cargo workspace with 13 crates:
  `maidan-types`, `maidan-store`, `maidan-bus`, `maidan-search`,
  `maidan-fsm`, `maidan-router`, `maidan-auth`, `maidan-artifacts`,
  `maidan-mcp`, `maidan-a2a`, `maidan-observability`, `maidan-cli`,
  `maidan-server`.
- Core domain schema 0001 (workspaces, members, channels, threads,
  messages, mentions, votes, references, artifacts, audit) in both
  Postgres and SQLite dialects.
- `maidan-store::Store` async trait, `PostgresStore`, `SqliteStore`,
  `Dialect::from_url` runtime routing, idempotent migration runner.
- `maidan-artifacts::ArtifactStore` async trait, `Sha256` newtype,
  `LocalFsStore` with sha-derived fanout paths, atomic tempfile-and-
  rename writes, content-addressed dedup.
- `maidan-server`: env-driven `Config`, `AppState` over
  `Arc<dyn Trait>` handles, `/health` endpoint returning structured
  `{status, db, storage, version}` body (200 on healthy, 503 on
  degraded with per-subsystem error string), axum + tower-http
  tracing layer, migration-on-boot.
- Production multi-stage Dockerfile (cargo-chef + distroless runtime),
  `Dockerfile.dev` (cargo-watch hot reload), `docker/Dockerfile.db`
  (pgvector + bundled schema).
- `compose.yaml` (prod-style) and `compose.dev.yaml` (hot reload).
- Full Kustomize manifest set: `k8s/base/` + `overlays/dev/` +
  `overlays/prod/`.
- Obsidian docs vault under [`docs/`](docs/) with Architecture,
  Roadmap, Conventions, Deploy, Glossary, Capabilities,
  Clusters/Cluster A, Retros/Cluster A.
- Integration test suite: testcontainers-backed Postgres roundtrip,
  SQLite roundtrip (shared assertion body), cross-dialect parity
  scenario, `/health` end-to-end test, LocalFsStore roundtrip +
  concurrency stress + proptest property tests.

### Changed

- Toolchain pinned at 1.88 (forced by transitive deps `icu_*` and
  `idna`).

### Security

- Established [SECURITY.md](SECURITY.md) reporting flow (GitHub private
  advisories preferred).
- `cargo deny` allowlist + `trufflehog` scan documented in
  [`docs/Conventions.md`](docs/Conventions.md) (CI gating lands in the
  next PR).
- `k8s/base/secret.example.yaml` documents the required Secret shape
  without storing values.

[Unreleased]: https://github.com/david-engelmann/maidan/compare/v1.4.0...HEAD
[1.4.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.4.0
[1.3.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.3.0
[1.2.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.2.0
[1.1.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.1.0
[1.0.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.0.0
[0.7.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.7.0
[0.6.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.6.0
[0.5.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.5.0
[0.4.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.4.0
[0.3.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.3.0
[0.2.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.2.0
[0.1.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.1.0
[0.0.1]: https://github.com/david-engelmann/maidan/releases/tag/v0.0.1
