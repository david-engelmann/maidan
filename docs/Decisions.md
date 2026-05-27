# Decisions

Architectural Decision Records (ADRs), inline. Each entry names a
decision, the alternatives that were considered, and what would have
to change for the decision to be revisited.

Decisions are append-only-ish: when a decision is reversed, the
original entry stays and a new entry below records the reversal and
why.

## Architecture

### `Arc<dyn Trait>` in `AppState`, not concrete backends

**Decision.** `AppState` carries `Arc<dyn Store>`, `Arc<dyn ArtifactStore>`,
`Arc<dyn EventBus>`, `Arc<dyn Search>`. Every handler clones the Arc;
the inner trait object handles the backend logic.

**Alternative.** Generic `AppState<S, A, B, X>` parameters threaded
through every handler.

**Why this:** the moment integration tests want to build the same
router with a tempdir artifact store + an in-memory bus + an
SQLite-backed search, the generic version needs 4 type parameters
everywhere. `Arc<dyn Trait>` makes the swap a one-line change and
keeps handler signatures readable.

**To revisit:** if dynamic dispatch becomes a measurable hot spot
under benchmarks (Cluster U).

### Subscriber-side filtering on the event bus

**Decision.** Both `InMemoryBus` and `PostgresBus` broadcast every
event to every subscriber; filtering happens client-side in the
subscriber's stream adapter (`stream.filter_map(...).filter`).

**Alternative.** Per-channel topic routing (Postgres NOTIFY channel
per workspace; tokio broadcast per filter group).

**Why this:** wire semantics stay identical across backends. No
backend-specific filter table to maintain. `PostgresBus` already
fans out to a process-local broadcast — adding per-subscriber
filtering at the receiver costs O(events × subscribers) but keeps the
mental model trivial.

**To revisit:** if a workspace fans out to >100 concurrent
subscribers and per-subscriber CPU on the filter becomes a hot spot.

### Bus `publish` failures never become 5xx

**Decision.** Every mutation handler calls `state.bus.publish(event)`
in a fire-and-forget pattern: errors are logged, not returned. The
store has already committed; a temporarily-unavailable bus should
not turn a successful mutation into a 500.

**Alternative.** Two-phase commit: roll back the store write if the
publish fails.

**Why this:** the bus is best-effort at-most-once until the persistent
event log lands (Cluster D). Forcing the store and bus into a single
transaction would require XA-style coordination across heterogeneous
backends and would create a new failure mode (bus unavailable →
all writes fail).

**To revisit:** when the persistent event log lands. At-least-once
semantics with a stored event row + outbox pattern would make this
trade-off pointless.

### Postgres transactional outbox (`v10.0.0`)

**Decision.** On Postgres, `append_event` inserts `maidan_events` and
`maidan_outbox` in one transaction. A background relay drains pending rows
and calls `PostgresBus::publish` (pointer NOTIFY). HTTP `publish` no longer
calls `bus.publish` directly on Postgres — the relay does after commit.
SQLite and `InMemoryBus` keep append-then-publish in-process.

**Alternative.** Continue append-then-publish in the handler; rely on
replay only when the process crashes between steps.

**Why this:** closes the crash window where a row exists but NOTIFY never
fires. NOTIFY remains fire-and-forget; relay retries can duplicate
publishes — subscribers must treat `log_id` as idempotent.

**To revisit:** end-to-end exactly-once or consumer dedup tables.

### Triggers maintain the lexical index; the indexer is for embeddings

**Decision.** Lexical (`tsvector` / FTS5) indexes are maintained by
DB triggers — synchronous on every write. The `maidan-search::Indexer`
task subscribes to the bus and is reserved for side effects that
shouldn't block the writer (embedding generation, mirror indexes).

**Alternative.** Indexer maintains every index asynchronously,
triggers do nothing.

**Why this:** synchronous lexical indexing makes every hit fresh.
The cost (one trigger per write) is negligible against the cost of
"is my message searchable yet?" UX. Embedding generation is
expensive enough that synchronous indexing would be prohibitive.

**To revisit:** if write latency on `maidan_messages` becomes a
problem, or if a non-text indexing pattern (e.g., named-entity
extraction) needs to run async.

### Unified `Search` trait with `Unsupported` per method

**Decision.** `Search` has both `search_messages` (lexical) and
`upsert_embedding` / `semantic_search` (vector). Backends that don't
implement a method return `SearchError::Unsupported`. Callers
discover capability via the error path, not a separate type.

**Alternative.** Split into `LexicalSearch` and `SemanticSearch`
supertraits.

**Why this:** the unified trait keeps `AppState::search:
Arc<dyn Search>` simple. Callers ask for the operation they want; the
backend says yes or excuses itself. Splitting into multiple traits
would require `AppState` to carry two handles and every call site to
know which one to use.

**To revisit:** if `Unsupported` errors become a common branch in
the HTTP / MCP layer, suggesting callers actually want capability
detection at compile time.

### Dialect-based backend routing in `main`

**Decision.** `Dialect::from_url(&database_url)` returns
`Postgres` or `Sqlite`. `main.rs` matches once on the dialect and
instantiates `(Store, EventBus, Search)` with the right backends.
The rest of the app sees only the trait objects.

**Alternative.** A single `sqlx::AnyPool`-based backend.

**Why this:** sqlx-Any doesn't cover every feature we use (e.g.,
typed Postgres NOTIFY payloads, pgvector). Branching once at boot
keeps every downstream call straightforward.

**To revisit:** if new backends arrive that have different
operational shapes (e.g., remote KV stores) and the matching balloons.

### MCP `McpServer` is transport-agnostic

**Decision.** `McpServer::handle(JsonRpcRequest) -> JsonRpcResponse`
is a pure function (modulo the Arc handles). The HTTP wrapper in
`maidan-server/src/mcp.rs` is 8 lines; a future stdio loop will be
the same shape.

**Alternative.** Couple `McpServer` to axum's `Request`/`Response`
types.

**Why this:** the JSON-RPC envelope split means there's nothing
transport-specific in the dispatcher. `Cluster H` adds an stdio
transport for desktop MCP clients; the dispatcher won't need to
change.

**To revisit:** if `McpServer` accumulates HTTP-specific assumptions
(e.g., streaming responses for `resources/subscribe`).

## Data

### Schema 0001's `tombstoned_at` columns (logical delete)

**Decision.** Every domain table has a nullable `tombstoned_at
TIMESTAMPTZ`. Tombstoned rows stay in the table; queries filter
`WHERE tombstoned_at IS NULL`. Hard deletes are reserved for GDPR
right-of-erasure (Cluster V).

**Alternative.** `DELETE` rows immediately.

**Why this:** audit trail; reversible moderation; the event log can
still reference tombstoned ids without dangling foreign keys.

**To revisit:** never. This is a load-bearing semantic.

### Postgres NOTIFY pointer delivery (`v7.0.0`)

**Decision.** On Postgres, `PostgresBus::publish` sends a small NOTIFY
payload `{"notify":"log_id_v1","log_id":N,"workspace_id":...}` when
`BusEnvelope.log_id > 0` (the normal path after `append_event`). The
background listener hydrates the row from `maidan_events` and fans out
a full `BusEnvelope`. Publishes with `log_id == 0` (synthetic / tests)
still use the legacy full JSON envelope and remain subject to the 7990-byte
NOTIFY cap.

**Alternative.** Continue shipping full envelopes on NOTIFY; or add an
outbox table for at-least-once delivery.

**Why this:** Cluster D made `maidan_events` authoritative; large events
no longer fail publish because of NOTIFY size. Hydration adds one PK read
per notification — acceptable vs multi-kilobyte JSON on the wire.

**To revisit:** outbox / guaranteed delivery remains a standing risk
(see [[Open Work]]). `InMemoryBus` stays full-envelope.

### Embedding dimension is 1024

**Decision.** `migrations/postgres/0003_embeddings.sql` declares
`embedding vector(1024)`. The Rust constant
`maidan_search::postgres::EMBEDDING_DIM` matches. Wrong-dimension
inputs error before SQL runs.

**Alternative.** Per-model embedding tables / dimension variations.

**Why this:** simpler to ship. 1024 is a reasonable default that
covers many small/medium models (OpenAI ada-002, voyage-3-small,
many open-source).

**To revisit:** when multiple models need to coexist in the same
deployment. Cluster D candidate.

### FTS5 is not contentless

**Decision.** SQLite FTS5 table is configured *with* a content
column (the default), not `content=''` (contentless).

**Alternative.** Contentless FTS5 with the `maidan_messages` table
as the external content source.

**Why this:** contentless FTS5 is append-only — DELETE from it is
forbidden, which breaks the tombstone trigger.

**To revisit:** if FTS5 storage overhead becomes prohibitive (it
duplicates the body text). On-disk size has not been an issue.

### `maidan_messages_fts_map` (UUID ↔ rowid bridge)

**Decision.** FTS5 requires an integer rowid; `maidan_messages.id`
is TEXT (UUID). A bridge table `maidan_messages_fts_map (rowid
INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT UNIQUE REFERENCES
maidan_messages(id))` translates between the two.

**Alternative.** Switch `maidan_messages.id` to INTEGER. Or use the
SQLite FTS5 hash trick.

**Why this:** the bridge is one table with two columns and a UNIQUE
constraint. Switching message ids to integers would require a
schema redesign and break Postgres parity.

**To revisit:** never. This is the cleanest way to bridge.

## CI + Tooling

### `cargo-deny` `wildcards = "deny"` + `allow-wildcard-paths = true` + `publish = false` everywhere

**Decision.** `deny.toml` denies wildcard version dependencies but
allows them for path deps; every workspace member sets
`publish.workspace = true` so the workspace-level `publish = false`
inherits.

**Alternative.** `wildcards = "warn"`. Or silently allow path deps.

**Why this:** `wildcards = "deny"` catches accidental `version = "*"`
declarations. `allow-wildcard-paths = true` only applies to crates
marked `publish = false` (path deps are forbidden on crates.io); the
workspace inheritance ensures every crate is correctly marked.

**To revisit:** when we want to publish some crates to crates.io
(maybe `maidan-types` and `maidan-mcp`). Then those crates need to
drop `publish = false` and stop using path deps for external
consumption.

### testcontainers use `pgvector/pgvector:pg17`, not `postgres:11`

**Decision.** Every Postgres testcontainer in the workspace runs
`Postgres::default().with_name("pgvector/pgvector").with_tag("pg17")`.

**Alternative.** Stock `postgres:17-alpine`. Skip vector tests on
plain images.

**Why this:** migration 0003 needs `CREATE EXTENSION vector`.
Pinning every test to the pgvector image keeps the suite consistent
and matches the `docker/Dockerfile.db` shipped image. The
performance overhead is negligible — the pgvector image is just
pg16/17 with the extension preinstalled.

**To revisit:** if pgvector ever stops shipping a docker image for
the Postgres major we want.

### `macos-13` for `x86_64-apple-darwin` builds

**Decision.** `release.yml` builds the `x86_64-apple-darwin` target
on `macos-13` (Intel runner), not `macos-latest` (arm64).

**Alternative.** Drop the target. Or build x86_64 on `macos-latest`
via cross-compile or Rosetta.

**Why this:** dropping the target hurts Intel Mac users (still common).
Cross-compile from arm64 is fragile. `macos-13` is the last Intel
default runner that GitHub still provides; it works without flags.

**To revisit:** when GitHub deprecates `macos-13`. At that point we
either drop the target or move to a build matrix that uses
`rustc --target` cross-compile from arm64 with sysroot setup.

## Workflow

### Admin-merge instead of local-first push

**Decision.** PRs are squash-merged via `gh pr merge --admin
--delete-branch`. Branch protection on `main` enforces the 5 CI
checks for everyone, including the maintainer; `--admin` bypasses
the required-review (since the maintainer can't review their own
PR) but does *not* bypass required-status-checks.

**Original direction (deferred).** Local-first push: nothing gets
pushed until `make ci` passes locally; remote `main` stays
buildable; no admin-merge.

**Why the reversal:** the user (sole maintainer) found local-first
slowed iteration without adding safety since they were the only
reviewer anyway. The CI-required-checks discipline replaces the
local-first discipline. Local CI is still encouraged but not
load-bearing.

**To revisit:** when a second human reviewer joins the project. At
that point, restore PR-review enforcement and drop the `--admin`
flag.

### Squash-merge only; PR body becomes the commit body

**Decision.** Merge commits and rebase are disabled at the repo
settings level. The PR title becomes the squash commit title; the PR
body (including the **mandatory** PR-level retro section) becomes
the commit body.

**Why this:** every commit on `main` carries its own retro inline.
`git log` is searchable. Cluster-level retros aggregate the per-PR
retros.

**To revisit:** never. This is load-bearing for the retro discipline.

### Annotated unsigned tags acceptable pre-1.0

**Decision.** Cluster tags are annotated (`git tag -a`) but not
signed. The user has not configured GPG/SSH signing as of `v0.1.0`.

**Alternative.** Block tagging until a key exists.

**Why this:** signing is a separate, mostly-one-time setup task.
Don't gate every release tag on it. Future tags can be re-issued
signed if needed.

**To revisit:** when a key exists.

### Semver-stable API from v1.0.0

**Decision.** From `v1.0.0`, HTTP route shapes and MCP tool/resource
names are treated as stable public API. Breaking changes require a
major version (`v2.0.0`). Pre-1.0 clusters could rename and delete freely.

**Why this:** agents and operators integrate against HTTP and MCP;
predictability matters once federation and UI exist.

**To revisit:** only via a deliberate `v2.0.0` program.

## Documentation

### Retro is mandatory; release tag never cut without it

**Decision.** Every cluster ends with a `[X.retro]` PR. The tag
gets cut only after the retro PR merges. The retro updates
`docs/Capabilities.md`, `CHANGELOG.md`, `README.md`,
`docs/Architecture.md`, `docs/Roadmap.md`, and
`docs/Retros/README.md` (the index).

**Why this:** declaring a cluster "done" requires writing the
retro, which forces explicit closure on what's deferred and what's
open. Skipping it is not allowed.

**To revisit:** never.

### Docs vault lives in `docs/` and uses Obsidian wikilinks

**Decision.** Project documentation is an Obsidian vault under
`docs/`. Notes use wikilink syntax (`[[Note Name]]`) for internal
references; filenames are Title Case with spaces.

**Alternative.** mdBook, Docusaurus, or plain Markdown without
wikilinks.

**Why this:** the maintainer uses Obsidian as their primary note-
taking tool. Wikilinks degrade gracefully on GitHub (which renders
them as bracketed text) without breaking the docs site. Cluster H
will pick a docs generator (mdBook / Docusaurus / VitePress) and
add a build pipeline that consumes the vault.

**To revisit:** in Cluster H when the docs site lands.

### OIDC human login deferred to `v2.0.0` (spike in `v1.4.2`)

**Decision.** `v1.4.0` ships bootstrap hardening (`MAIDAN_BOOTSTRAP`) and an
OIDC **design document** ([[OIDC]]) only. Runtime OIDC login, session cookies,
and identity tables land in **`v2.0.0`**.

**Alternative.** Ship OIDC in `v1.4.0` alongside bootstrap gating; or defer
both doc and code to `v2.0.0`.

**Why this:** OIDC adds a new trust boundary (browser sessions, IdP claims,
CSRF/PKCE) on top of the stable bearer-token API. A minor release should not
break MCP/WS clients or semver-stable HTTP auth. The spike unblocks planning
and threat-model updates without half-implemented login.

**To revisit:** if a deployment needs browser login before `v2.0.0`, use an
external reverse proxy (OAuth2 Proxy) in front of `/ui/` only — documented in
[[OIDC]] as a stopgap, not a supported Maidan API.
