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

### Transactional outbox (`v10.0.0` Postgres, `v14.0.0` SQLite)

**Decision.** On Postgres and SQLite, `append_event` inserts `maidan_events` and
`maidan_outbox` in one transaction. A background relay drains pending rows after
commit. On Postgres the relay calls `PostgresBus::publish` (pointer NOTIFY); on
SQLite it calls `InMemoryBus::publish` (in-process fan-out). HTTP `publish` does
not call `bus.publish` directly when outbox relay is enabled — the relay does.

**Alternative.** Continue append-then-publish in the handler; rely on
replay only when the process crashes between steps.

**Why this:** closes the crash window where a row exists but subscribers never
see the event. Postgres NOTIFY remains fire-and-forget; relay retries can duplicate
publishes — subscribers must treat `log_id` as idempotent.

**To revisit:** end-to-end exactly-once or consumer dedup tables.

### Outbox quarantine after max relay attempts (`v12.0.0`)

**Decision.** After `MAIDAN_OUTBOX_MAX_ATTEMPTS` (default **16**) failed relay
publishes, the row is marked `quarantined_at` and excluded from relay batches.
Operators recover manually (clear quarantine, adjust `attempts`, or re-append);
rows are never auto-deleted.

**Alternative.** Retry forever; or delete quarantined rows automatically.

**Why this:** poison payloads or prolonged bus outages must not spin the relay
or inflate `maidan_outbox_pending` indefinitely. NOTIFY remains at-least-once;
quarantine stops relay only, not subscriber replay.

**To revisit:** admin replay API; consumer dedup tables (Cluster 13).

### Delivery cursors (`v13.0.0`)

**Decision.** Postgres stores `maidan_delivery_cursor (consumer_id, workspace_id) →
last_delivered_log_id`. Subscribe clients may pass `consumer_id` on WebSocket and MCP
SSE; the server uses `max(after_id, cursor)` for replay and advances the cursor on
each delivered `log_id`. Federation ingest advances `federation:{peer_id}` after
successful local append.

**Alternative.** Rely only on client-side dedup and `resume_token` without server
ledger.

**Why this:** reduces duplicate delivery on reconnect and documents a durable
watermark per consumer. NOTIFY remains at-least-once; cursors are monotonic hints,
not exactly-once guarantees.

**To revisit:** SQLite cursors; HTTP admin to reset cursors.

### Triggers maintain the lexical index; the indexer is for embeddings

**Decision.** Lexical (`tsvector` / FTS5) indexes are maintained by the
DB synchronously on every write. The exact mechanism is dialect-specific:
Postgres uses a `GENERATED ALWAYS … STORED` `search_vec` column (GIN-indexed),
SQLite uses FTS5 triggers (`maidan_messages_fts_insert/_update/_tombstone`).
(The title says "triggers" as shorthand for "the DB keeps it current, not the
indexer"; on Postgres it is a generated column.) The `maidan-search::Indexer`
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
`maidan-server/src/mcp.rs` is a thin shim (~two dozen lines, after later
capability/quota plumbing); the stdio loop added in `Cluster H`
(`maidan mcp-stdio`) is the same shape.

**Alternative.** Couple `McpServer` to axum's `Request`/`Response`
types.

**Why this:** the JSON-RPC envelope split means there's nothing
transport-specific in the dispatcher. `Cluster H` adds an stdio
transport for desktop MCP clients; the dispatcher won't need to
change.

**To revisit:** if `McpServer` accumulates HTTP-specific assumptions
(e.g., streaming responses for `resources/subscribe`).

### MCP `resources/subscribe` ships stdio-first (`v15.0.0`)

**Decision.** Implement `resources/subscribe` and `resources/unsubscribe`
on the JSON-RPC dispatcher, and deliver
`notifications/resources/updated` on stdio transport in the same process.
`POST /mcp` remains request/response-only for now.

**Alternative.** Implement streamable HTTP and stdio together in one cluster.

**Why this:** desktop MCP clients are already stdio-first, and this closes
the long-standing subscription deferral without coupling to HTTP streaming
infrastructure.

**To revisit:** streamable HTTP parity and broader resource update fan-out.

### MCP resource notifications on HTTP SSE (`v16.0.0`)

**Decision.** Share one `McpServer` per process in `AppState`; fan-out
`notifications/resources/updated` on a tokio broadcast channel; expose
`GET /mcp/notifications` as an SSE stream of JSON-RPC notification lines.
`POST /mcp` stays one-request-one-response.

**Alternative.** Full MCP streamable HTTP session multiplexing on a single
connection.

**Why this:** closes HTTP parity for the Cluster 15 subscribe surface without
replacing `/mcp/stream` or implementing the full transport spec.

**To revisit:** session-scoped MCP servers per bearer token; broader resource
fan-out beyond `post_message`.

### Resource notifications ride a dedicated NOTIFY channel (`v102.0.0`)

**Decision.** MCP resource-update notifications fan out across replicas on a
**dedicated** `maidan-bus::ResourceNotifier` channel (Postgres `LISTEN`/`NOTIFY`
on `maidan_resource_updated`), carrying the `maidan://` URIs a mutation touched.
The originating replica publishes the *unfiltered* URI set; every replica's
listener applies its own local subscription filter and delivers to its SSE
subscribers. The inline tool-call response (`take_pending_notifications`) stays
local and synchronous.

**Alternative.** Re-derive resource URIs from the existing domain `Event` stream
on each replica (the event bus already crosses processes), avoiding a second
NOTIFY channel.

**Why this:** not every resource fan-out maps 1:1 to a domain `Event`
(`pin_message`, `cast_vote`, reactions, references), so event-inference would
miss notifications. Publishing the URIs the existing `uris_for_*` logic already
produces is exact. A single delivery path (the originator also delivers via its
listener loop) means no de-duplication. At-most-once delivery matches the bus;
a dropped notification is reconciled by the client re-reading the resource.

**To revisit:** cross-pod migration of in-flight streamable sessions (currently
pod-pinned); collapsing the two NOTIFY channels if the URI set ever becomes a
strict function of events.

### Distributed presence: heartbeat + TTL over NOTIFY (`v103.0.0`)

**Decision.** Presence/typing/roster cross replicas via a **dedicated**
`maidan-bus::PresenceNotifier` channel (`maidan_presence`) carrying a typed
`PresenceEvent`. Each replica keeps a **merged, TTL-expiring** remote view; a
periodic **heartbeat** re-announces local members (refreshing remote TTLs) and a
sweep expires stale ones. TTL is **receiver-stamped** (each replica uses its own
clock on receipt — no cross-pod wall-clock). Heartbeats refresh `last_seen`
silently; only genuine changes fan out to subscribers (`PresenceEvent.heartbeat`
+ dedupe). Wired only in **Postgres NOTIFY mode**; single-process keeps the
legacy local-only hub.

**Alternative.** A shared `maidan_presence` table upserted on every heartbeat
(durable, queryable), or Redis TTL keys + pub/sub.

**Why this:** a presence table would mean a DB write per member per heartbeat
(write amplification); Redis would be a new hard dependency for multi-replica
presence. The NOTIFY + per-replica TTL view reuses Cluster 102's substrate with
no new infra. Unlike the resource notifier (attached in-memory everywhere),
presence is gated to Postgres+NOTIFY: its heartbeat task is pure overhead in a
single process, where the legacy local broadcast is already correct.

**To revisit:** Redis-backed presence if heartbeat NOTIFY volume becomes a
bottleneck at high replica/member counts; persistent "last seen".

### Durable ephemeral state: persist, don't replicate (`v104.0.0`)

**Decision.** App OAuth authorization codes and reindex job status move from
per-replica memory into the store (`maidan_oauth_codes`, `maidan_reindex_jobs`),
not onto a NOTIFY channel or a cache. Codes are stored as a SHA-256 hash with a
short TTL; single-use is enforced atomically by
`DELETE … WHERE code_hash = ? AND expires_at > ? RETURNING …` (no read-then-delete
race). The reindex `ReindexJob` model moves to `maidan-types` so store and server
share one definition.

**Alternative.** Fan the state over NOTIFY like Clusters 102/103, or keep an
in-memory map plus sticky-session load balancing.

**Why this:** unlike presence/resource updates — *ephemeral signals* with nothing
to read back, which is exactly what NOTIFY is for — codes and job status are
values a later request must *read*. Durability and any-replica visibility then
fall out of a single store write; a NOTIFY channel would still need a backing
store for the read, and sticky sessions don't survive a pod restart. Atomic
`DELETE … RETURNING` makes single-use a property of the database, not the handler.

**To revisit:** distributed reindex *execution* (a job whose owner dies stays
`Running`) — deferred to the Phase XXII work-scheduling cluster; a periodic
purge of expired/idle rows if volume grows.

### Serialize boot migrations with an advisory lock (`v105.0.0`)

**Decision.** `run_postgres_migrations` holds a Postgres **session advisory
lock** (`pg_advisory_lock`) while applying. When several replicas boot against a
fresh or upgrading database they would otherwise run non-transactional DDL
concurrently — notably `CREATE EXTENSION`, which fails with a `pg_extension`
unique violation even with `IF NOT EXISTS` (the existence check is not atomic
against a concurrent create). The first replica migrates; the rest block, then
observe the migrations applied and no-op.

**Alternative.** A dedicated migration `Job`/init-container that runs before
replicas start (Helm pre-install hook); or `pg_advisory_xact_lock` with all
migrations in one transaction.

**Why this:** keeps the simple "migrate on boot" operational model (no extra
deploy step) while making it correct under N replicas. The distroless runtime
image has no shell, so gating replica start order on an HTTP healthcheck via
`depends_on` wasn't available; the advisory lock needs nothing but the database.
One giant transaction would change the per-migration commit semantics and breaks
on any future non-transactional step (e.g. `CREATE INDEX CONCURRENTLY`).

**To revisit:** a pre-deploy migration Job if/when migrations grow long enough
that holding the lock during a rollout meaningfully delays replica readiness.

**Updated (`v107.0.0`):** when `MAIDAN_DB_STATEMENT_TIMEOUT_MS` is set, the cap
is applied to every pooled connection via `after_connect` — which would
otherwise kill the advisory-lock *wait* a booting replica performs while another
replica migrates. The migration session now resets `statement_timeout = 0` on
its own connection before acquiring the lock (unconditional; a no-op when no cap
is configured), so pool tuning and boot-migration serialization compose cleanly.

### Bulk reads for context assembly; the store grows batched accessors as call sites need them (`v106.0.0`)

**Decision.** Context builders read in batches, not one query per row. The
`Store` trait gains concrete `…_many` / `…_for_workspace` accessors
(`list_threads_for_workspace`, `list_references_from_many`,
`list_message_edits_for_messages`) as specific N+1 call sites demand them —
Postgres binds id arrays (`= ANY($1)`), SQLite expands chunked `IN (?, …)`. New
batched methods are added only when a hot path needs one, not speculatively.

**Alternative.** A generic query-builder / DataLoader-style abstraction over the
store; or a request-scoped cache.

**Why this:** concrete accessors keep the store's runtime-checked-SQL model
(no query-builder indirection, both dialects explicit and testable) and stay
honest about cost — each method is one statement with a known plan. A caching
layer trades correctness for speed and is a separate, later concern. A 40-message
thread now issues the same query count as a 3-message one (`context_query_count_e2e`).

**To revisit:** if the number of batched accessors grows unwieldy, reconsider a
narrow loader abstraction; batch artifact-metadata reads if they become hot.

### SQLite semantic search without `sqlite-vec` SQL (`v18.0.0`)

**Decision.** Store 1024-dim float32 embeddings in `maidan_message_embeddings`
and rank with cosine similarity in Rust inside `SqliteSearch::semantic_search`.

**Alternative.** Load `sqlite-vec` via `sqlite3_auto_extension` and use
`vec_distance_cosine()` in SQL.

**Why this:** the `sqlite-vec` crate did not register with sqlx's libsqlite3
(`no such function: vec_distance_cosine`); alpha crate builds were also brittle.
Dev parity matters more than SQL-side distance for SQLite.

**To revisit:** wire `sqlite-vec` when sqlx/extension linkage is reliable.

**Superseded by** “sqlite-vec via sqlx `lock_handle`” (`v48.0.0`).

**Storage restructured** at `v47.0.0`: the single `maidan_message_embeddings`
table became a registry (`maidan_embedding_models`) plus one table per model
(`maidan_emb_hash_v1`, …); see
[Architecture](Architecture.md#per-model-embeddings-at-v4700).

### sqlite-vec via sqlx `lock_handle` (`v48.0.0`)

**Decision.** Load `sqlite-vec` statically on each sqlx SQLite connection via
`after_connect` + `SqliteConnection::lock_handle`, then rank with
`vec_distance_cosine()` in SQL. Rust brute-force cosine remains as fallback when
the extension is unavailable.

**Alternative.** Keep brute-force only; or use `vec0` virtual tables (schema churn).

**Why this:** sqlx 0.8 exposes `lock_handle` for per-connection extension init;
`sqlite-vec` 0.1.9 links reliably as `sqlite_vec0`. SQL-side distance restores
`LIMIT` pushdown without fetching all embeddings.

**Production scale:** Postgres + pgvector HNSW remains the production path;
SQLite is dev parity.

### Unified `SearchHit.score` (`v48.0.0`)

**Decision.** Add `score` in `[0, 1]` alongside backend-specific `rank`.
Semantic: `score = rank`. Lexical: min-max normalize ranks within the response.

**Alternative.** Normalize ranks globally across backends (needs calibration data).

**Why this:** clients can compare hit quality across Postgres and SQLite within
one mode without parsing backend-specific `rank` ranges.

## Security

### Revoking a token revokes everything derived from it (`v401.3.0`)

**Decision.** `maidan_api_tokens` gains a `parent_token_id`, attenuation records
it, and `revoke_api_token` revokes the whole subtree — transitively, not one
level.

**Why cascade rather than mark.** Cluster 397.7 established the principle: *a
derived token inherits every limit the parent carried*. It fixed exactly this
shape for app installations and per-token quotas, because re-issuing was
otherwise a way to shed a bound. Revocation is the ultimate limit, and it was the
dimension still leaking — the parent link lived only in audit metadata, so
nothing could traverse it. The practical case decides it: you revoke a parent
because it leaked, and whoever held it could have minted children from it. Those
children are equally compromised.

**Why at revoke time, not auth time.** Writing `revoked_at` across the subtree is
one traversal. Checking the ancestor chain on every request would put a recursive
query in the hot auth path, which is the wrong place to spend. Attenuation
requires a *live* parent, so a child cannot appear after its parent is revoked —
there is no window for write-time cascade to miss.

**Why not a field on `NewApiToken`.** That struct is constructed at 109 sites,
100 of them tests, and only the attenuation path has a parent. A field would have
been a hundred mechanical edits serving one caller — the ripple that Cluster 173
hit on `NewMessage`. `create_attenuated_api_token` has zero blast radius.

**`ON DELETE SET NULL` on the parent FK**, never CASCADE: deleting a parent row
should sever the link, not delete its children's rows. Workspace teardown already
cascades through `workspace_id`.

**Already-revoked rows are skipped**, so a child revoked earlier keeps its own
timestamp when an ancestor is revoked later. "When was this killed" stays true.

**Trade accepted.** Revoking a token now has a blast radius its holder may not
have in mind. That is the intended meaning of a kill switch, and the alternative
— a compromised credential's descendants surviving it — is worse.

### Separation of duties reads a worker ledger, not the live assignee (`v401.1.0`)

**Decision.** A durable, append-only `maidan_thread_workers` table records every
member who has ever held a thread. Both governance gates (Cluster 375 reviews,
Cluster 385 land gate) ask *"has this reviewer ever worked this thread?"* rather
than *"is this reviewer the current assignee?"*.

**Why the live column could not stay the input.** Both gates tested
`thread.assignee_id`, and releasing a claim sets it to `NULL` — so the exclusion
became vacuous at exactly the moment someone wanted it to be. Do the work,
release the claim, approve your own work as a qualifying third party. The gate
still ran; it just had nothing left to compare against.

**Why not the event log.** Assignment history *is* recorded there, and reading
it would need no new table. But Cluster-186 retention prunes the event log, and
a security control cannot depend on evidence that ages out. A gate that weakens
after ninety days is a gate with a calendar.

**Why not a `last_worked_by` column.** It is smaller and it is wrong: it
remembers only the most recent holder. A bearer token is act-as-any by design
(the orchestrator model, Cluster 202), so an agent could claim *as* another
member, overwrite the column, and approve. A ledger accumulates, and nothing in
the API can un-write a row.

**Why the write lives in `append_assignment_event`.** Every event-emitting
assignment path funnels through it, so that is one site per backend instead of
three — and a separation-of-duties control that one call site can forget is not
a control. The three non-event variants (`assign` / `claim` / `claim_next`,
reachable through the `Store` trait) were converted to transactions and record
there too. The write is always on the assignment's own transaction: a ledger row
lost while the assignment commits fails **open**, which is the defect itself.

**What this does not fix.** The ledger cannot reconstruct releases that already
happened — the migration backfills only the current holder of each thread, so it
makes the gate no weaker than before and no stronger about the past. It also
does not address a *genuinely* colluding pair of members; separation of duties
never did.

**Trade accepted.** One row per (thread, member) that is never pruned except
with its thread, in exchange for a gate whose exclusion cannot be cleared by the
person it excludes.

### A workspace handle is a display label, not an address (`v398.7.0`)

**Decision.** Maidan will **not** resolve a handle to a workspace. There is no
`GET /rooms/:handle`, and the store's reverse lookup (`workspace_id_for_handle`,
shipped unused in Cluster 395) is removed. A handle is a renameable, unique,
human-readable label shown on the room card — nothing addresses by it.

**Context.** Cluster 395 shipped the handle table, `PUT`/`GET
/workspaces/:id/handle`, and a both-backend `workspace_id_for_handle` with a
test. No route ever called it. That reads as a half-built feature — you can name
a workspace and read the name back, but nothing finds a workspace *by* name — so
the obvious instinct is to add the missing endpoint.

**Why the endpoint should not exist.** It has no consumer in Maidan's actual
model, and it is not free:

- **A token already carries its workspace.** Tokens are minted per workspace
  through `POST /workspaces/:wid/members/:mid/tokens` and the resolved
  `AuthContext` holds `workspace_id`. A caller holding a token has nothing to
  look up.
- **There is no self-serve join.** Nothing lets a stranger discover a workspace
  and request access, which is the flow a public handle serves elsewhere
  (`acme.slack.com`). An admin mints the token out of band; the client is
  configured with it.
- **`maidan://` URIs deliberately use the UUID.** Cluster 395 chose that so a
  rename cannot break a stored citation, pin or export. The addressable identity
  is the id *by design* — a resolver would introduce a second, weaker one.
- **It would be a tenant-existence probe.** `/.well-known/maidan-room` already
  refuses to list tenants ("handles are aliases, not a public directory").
  Resolution is not enumeration — you must guess the handle — but it answers
  "does `acme` use this instance?" to anyone who asks, with no compensating use.
- **Authenticating it does not rescue it.** Scoped to the caller's own workspace
  it only restates what the token already says; unscoped-but-authenticated it
  still probes, just with a token.

**Uniqueness is unaffected.** It comes from the table's constraint, surfaced as
`StoreError::Conflict` by `set_workspace_handle` — never from a reverse lookup.

**Revisit if** a pre-authentication flow appears that genuinely needs it: a
self-serve join, an instance-level workspace picker, or a hosted front door where
a client arrives knowing only a name. Resolution should then be designed *with*
that flow — rate-limited, returning the room card rather than membership, and
with the enumeration trade taken knowingly — rather than added now because a
store method looked lonely.

### WASI slash handlers run on **wasmi**, not wasmtime (Wave 3 #36)

**Decision.** Use **[wasmi](https://github.com/wasmi-labs/wasmi)** — a pure
interpreter — as the WASI engine for slash handlers. Not wasmtime, despite
wasmtime being the Bytecode Alliance reference implementation and the default
choice for most embedders.

**Context.** Cluster 396 landed the ABI (`crates/maidan-types/src/wasi.rs`) and
nothing else: `SlashHandlerKind::wasi` is registrable on both write surfaces and
every dispatch returns `wasi_runtime_unavailable`. The ABI already pins the
shape the engine has to satisfy — WASI preview 1 (`wasi_snapshot_preview1`, any
other import module is a banned outbound host call), fuel (`WASI_DEFAULT_FUEL`
25M, `WASI_MAX_FUEL` 100M) and a linear-memory cap (16 MiB default, 64 MiB hard).
It is also already engine-agnostic in wording: *"interpreter / cranelift fuel
units"*. Both engines satisfy preview 1; wasmi does so through `wasmi-wasi`.

**Why not wasmtime.** This is a code-execution surface accepting untrusted guest
modules, in a multi-tenant server. The threat that matters is sandbox escape, and
wasmtime's escapes come from its **compilers**:

- [Wasmtime's 9 April 2026 advisories](https://bytecodealliance.org/articles/wasmtime-security-advisories)
  were the largest set in the project's history — triple the total for all of
  2025, and double the number of Critical advisories ever published. Four issues
  in the Winch backend, two in Cranelift.
- [RUSTSEC-2026-0095](https://rustsec.org/advisories/RUSTSEC-2026-0095.html)
  (CVE-2026-34987, CVSS 9.0 Critical) is a sandbox-escaping memory access via
  the Winch backend.
- RUSTSEC-2026-0269 is a **WASI filesystem sandbox escape** through trailing
  slashes on paths and symlinks (CVSS 8.8), alongside RUSTSEC-2026-0268
  (guest-controlled host allocation).
- Earlier: [RUSTSEC-2026-0006](https://rustsec.org/advisories/RUSTSEC-2026-0006)
  (out-of-sandbox load via `f64.copysign` on x86-64) and
  [a longer tail](https://rustsec.org/packages/wasmtime.html).

**In fairness to wasmtime:** Cranelift on x86-64 — the default, most-scrutinised
backend — was *not* affected by the two Critical escapes; Winch was. Choosing
wasmtime with Cranelift is a defensible position, and this ADR should not be read
as "wasmtime is unsafe." It is a statement about which *class* of bug we want to
be exposed to at all. A JIT has a codegen attack surface; an interpreter does not
have one to have bugs in.

[wasmi](https://wasmruntime.com/en/runtimes/wasmi) has no RUSTSEC advisories, has
been audited twice, and is the execution engine for Polkadot/Substrate — an
adversarial, high-value environment whose entire threat model is "run untrusted
code submitted by strangers."

**The deciding argument is fuel stability, not the advisory count.**
`WASI_DEFAULT_FUEL` is a documented, operator-tunable constant. Under wasmtime it
counts Cranelift fuel, whose cost per unit of work can move between versions — so
the same guest can start exceeding an unchanged cap after a routine dependency
bump, and the operator's number silently means something different.
[Wasmi 2.0 ships *stable* fuel metering](https://wasmi-labs.github.io/blog/posts/wasmi-v2.0/):
metered fuel per unit of execution is held constant across versions. For a
published cap that operators tune, that is a correctness property.

**What we give up.** Interpretation is materially slower than JIT. That is
acceptable here and would not be everywhere: a slash handler is *the tool, not an
agent runtime* — it parses arguments, does something small, returns JSON — it is
already bounded by fuel and by the existing slash `DISPATCH_TIMEOUT`, and it runs
on an interactive path where a guest needing JIT-class throughput is misplaced by
design. We also give up the Component Model / WASI p2 trajectory, which the
preview-1 ABI has already declined for now. Revisit if a real handler is
compute-bound rather than IO-shaped.

**Secondary benefits.** A much smaller dependency tree (no Cranelift, no object,
no codegen stack) — less `cargo-deny` surface on a project that has already been
bitten twice by transitive advisories, and a faster build on a workspace where
the release arm64 leg has been the pole before.

**Revisit if:** a handler's workload becomes genuinely compute-bound; wasmi's
audit/advisory record degrades; WASI p2 / the Component Model becomes required by
the ABI; or wasmtime ships an interpreter-only, codegen-free configuration that
carries the same "no compiler in the trust path" property.

### Postgres Row-Level Security assessed, deferred; app-layer RBAC is authoritative (`v216.0.0`)

**Decision.** Do **not** adopt Postgres Row-Level Security (RLS). Tenant isolation
and channel/thread access control stay enforced entirely at the application layer —
the `maidan_auth::access` helpers (`ensure_channel_access` / `ensure_thread_access`
/ `ensure_message_access` and the `can_access_*` / `*_deny_set` filters), applied on
every REST + MCP content route, the WS/MCP subscribe grants, the search + context
filters, and the federation/A2A ingress (the Cluster 160–165 arc plus 179–183,
202–204). This ADR is the Program-A "RLS spike": it records the assessment and the
conditions under which RLS would be revisited.

**How RLS would work here.** RLS keys each row-visibility policy on a
per-connection session GUC — e.g. `SET LOCAL app.current_workspace = '<uuid>'` at
the start of a request's transaction, with policies like
`USING (workspace_id = current_setting('app.current_workspace')::uuid)` on every
tenant-scoped table. The database then denies cross-tenant rows even if an
application query forgets a `WHERE workspace_id = …`.

**Alternatives considered.**

1. **Full RLS.** Enable RLS on every tenant-scoped table + thread the current
   workspace through a per-request GUC.
2. **RLS on a subset** (e.g. only `maidan_messages`).
3. **No RLS — app-layer RBAC only** (chosen).

**Why defer.**

- **The connection pool has no per-request tenant binding.** The `PgPool` is a
  shared 16-connection pool whose only per-connection setup is `statement_timeout`
  (`main.rs` `after_connect`). `SET LOCAL` is transaction-scoped, so RLS would
  require wrapping **every** read in a request-bound transaction that first sets the
  GUC — today most `Store` reads run directly on `&pool` outside any transaction.
- **The `Store` trait is workspace-agnostic.** Its methods take entity ids, not a
  request/workspace context; RLS needs that context at query time. Supplying it
  means threading a "current workspace" (and the bypass/orchestrator distinction)
  through every `Store` method and both backends — a large, cross-cutting refactor.
- **SQLite has no RLS.** The store is dual-backend with enforced parity (both
  backends run the same suite). RLS would be Postgres-only, so the SQLite path would
  still rely solely on app-layer RBAC — an asymmetry that weakens the "both backends
  are equivalent" guarantee the project leans on.
- **The bearer/orchestrator model is cross-workspace by design.** A bearer token is
  an act-as-any orchestrator (Cluster 202–203); a single `current_workspace` GUC
  doesn't fit an operation that legitimately spans workspaces without per-operation
  GUC juggling or a broad bypass role — which reintroduces the app layer as the real
  policy.
- **It duplicates an already-comprehensive, tested control.** The app-layer RBAC
  gates reads, writes, events, management, references, artifacts, search, and
  federation ingress, with e2e coverage. RLS would be defense-in-depth *over* that —
  real value only against an app-layer bug, at a high refactor + parity cost.

**Why this (app-layer only).** The authoritative control is where the domain
context lives (auth + entity graph), it is uniform across both backends, and it
already covers every surface. RLS's marginal benefit (catching a missed `WHERE`)
does not justify a pool + `Store`-context refactor that only protects the Postgres
half.

**To revisit** — adopt RLS if **any** of these hold: (a) a multi-tenant compliance
requirement mandates database-enforced isolation; (b) the `Store` gains a
per-request context object (for read-replica routing or query tracing) that could
carry the workspace GUC cheaply — at which point RLS becomes incremental; (c)
Postgres becomes the sole supported backend, removing the parity concern. If
adopted, start with `maidan_messages` + `maidan_channels` behind a
`SET LOCAL`-in-transaction wrapper and a `bypass` role for orchestrator/federation
paths, and keep the app-layer checks as the primary control.

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

### At-least-once delivery via cursor reconciliation + a time-based stability horizon

**Decision (Cluster 125).** Live subscription stays the low-latency optimistic
path (broadcast bus, monotonic `watermark` per stream — which already dedups
re-published / NOTIFY-duplicated `log_id`s). Completeness is provided by a
**reconcile loop**: for `workspace + consumer_id` subscriptions, a periodic timer
(and a NOTIFY hint) replays `list_events_after_stable(cursor, now - W)` in strict
`id` order and advances the durable `delivery_cursor`. A row is **stable** only
once its DB insert time (`maidan_events.inserted_at`, set by the app at append —
distinct from the caller-supplied `occurred_at`) is older than the window `W`.

**Why this.** The real delivery hole was never duplicates (the watermark + the
`delivery_cursor` floor already handle those) — it was *silent gaps*: an event
whose `log_id` arrives after a higher one was already delivered (a failed outbox
row retried later, or a late-committing `BIGSERIAL`) is `<= watermark` and
dropped, and replay only fires on broadcast `Lagged`. Gating the cursor on a
stability horizon guarantees that, under "no insert transaction outlives `W`",
no lower `id` can still commit and be stranded behind the cursor — so the
reconcile loop eventually delivers every committed row exactly once per consumer.

**Alternatives.**
- *Commit-sequence column* (assign a monotonic commit-order value at commit and
  consume strictly by it): truly strict with no time assumption, but needs a
  migration + insert-path change and is awkward on SQLite (no clean commit-time
  sequence). Rejected as too invasive for the gain.
- *Contiguity detection* (`log_id` skipped ⇒ gap): wrong — filtered streams and
  the global serial legitimately skip ids.
- *Pure live + client dedup* (status quo): leaves the silent-gap hole.

**Cost.** A backfill-latency floor of `W` (default small, tunable via
`MAIDAN_DELIVERY_STABILITY_SECS`); the optimistic live path is unaffected, so
steady-state latency is unchanged. Not strict against a pathologically long
(`> W`) insert transaction — accepted, and documented.

**To revisit:** if sub-`W` completeness is required, or if a long-transaction
workload makes `W` impractical — then the commit-sequence column (or logical
decoding) becomes warranted.

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

## Product scope

### Fidelity + context flagship arc — the optional tail is declined (`v331.0.0`)

**Decision.** The fidelity + context flagship arc (Clusters 319–330) is **complete**.
Its explicitly-optional tail is **declined**, not deferred — the value each item promised
is already deliverable by composing shipped primitives, and adding bespoke surfaces for it
would violate the arc's locked anti-goals ("a room, not a brain; perfect at what it does,
not more"). This ADR records what was declined and why, so a future research round starts
from a clean, deliberate baseline rather than an implicit backlog.

**What shipped (the arc).** Typed reference relations + reverse/by-type queries (319–320);
shared glossary — store → REST/MCP → grounded into the context pack (321–323); optional vote
`confidence` for weighted consensus (324); agent conventions — decision records, supersession,
grounding acks, as docs + a proving e2e with zero server code (325); as-of context replay from
the immutable event log (326); seed-from-message over REST + MCP (327–328); immutable
content-addressed context snapshot artifact over REST + MCP (329–330).

**Declined tail + why each is already covered.**

1. **Seed `pack` / `prefix` inclusion.** A seed can already start from a frozen context: an
   agent calls `POST /threads/:id/context/snapshot` (329) — optionally `?as_of=<event>` (326)
   for the prefix-before-the-tangent — then `POST /messages/:id/seed` (327) and carries the
   snapshot sha. A dedicated `pack`/`prefix` inclusion mode is a *convenience wrapper* over
   snapshot + seed + as-of, not new capability; leaving the composition to the agent keeps
   the seed endpoint a single clean gesture.
2. **`WorkSeeded` single-signal event.** A seed already emits `ThreadCreated` +
   `ReferenceAdded` (the `seeded_from` edge). A watcher gets the full "a branch spawned from
   message X" signal by correlating those two; a third event kind would add the 11-site
   EventKind drill for a filter convenience, with no new information.
3. **Flow / setup template (`structure_only` clone).** Cloning a workspace's setup
   (channels/skills/schedules/DAG skeleton) is covered by the shipped workspace export (187) +
   import-remap (269–270): export a source workspace, prune content, import. A dedicated
   `structure_only` export filter is the arc's flagged highest-scope-creep item and the room
   must never score which template is "better" (a locked anti-goal); declined until a research
   round shows concrete demand.

**To revisit.** A future research round may re-open any of these with evidence of real
demand. `pack`/`prefix` inclusion is the most likely candidate (pure convenience, low risk);
a `structure_only` export filter is the least (scope-creep toward a template product). None
is a correctness or capability gap today.
