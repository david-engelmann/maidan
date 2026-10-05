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

**To revisit:** admin replay API; consumer dedup tables.

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

**Why this:** closes HTTP parity for the subscribe surface without
replacing `/mcp/stream` or implementing the full transport spec.

**To revisit:** session-scoped MCP servers per bearer token; broader resource
fan-out beyond `post_message`.

### MCP resource subscriptions belong to a caller and session

**Decision.** A `resources/subscribe` belongs to one subscriber: the caller's
identity (workspace, member, actor, credential, grant) plus the MCP session it
arrived in — a `2024-11-05` `Mcp-Session-Id`, the stdio process, or, on the
stateless transports, the credential itself. A notification is addressed to a
subscriber and only that subscriber's listeners forward it. Every resource
update carries the workspace it happened in (across replicas too), and each
delivery re-checks the subscriber's access to the resource; losing access ends
the subscription. A session's subscriptions end with it; a stateless caller's
end after the session TTL with no open listener on any replica; one
subscriber may watch at most 1024 resources. A streamable session is open only
to the caller that opened it. A stateless caller's subscriptions are kept in
the database (`maidan_mcp_resource_subscriptions`, keyed by the caller's full
principal and scoped to its workspace), so its subscribe and its listener may
land on different replicas: on each update the replica holding the listener
looks up that caller's subscriptions for the update's workspace and delivers.
A replica with an open stateless listener extends that caller's subscriptions
several times per TTL; when the listener closes or its replica dies, nothing
extends them and they lapse. Session subscriptions stay in the process that
holds the session.

**Alternative.** Keep one set per workspace (the first fix) and filter at the
listener by workspace; or require a server-minted listener id on every
stateless subscribe.

**Why this:** per-workspace keying still told every member about private
threads one member watched, let one member's unsubscribe silence the rest, and
could not scope a content-addressed artifact, whose URI names a resource in
every workspace that uploaded the same bytes. A listener id would add a handle
the stateless revisions do not define; the credential is already the stateless
caller's identity.

A table, not a NOTIFY announcement of each subscribe: an announcement reaches
only the replicas running at that moment, so a replica that starts later, or
the one a client reconnects to after its replica died, would not know it. A
row every replica reads survives both. Delivery costs one indexed lookup per
update batch on a replica with a stateless listener open, and none elsewhere.

**To revisit:** `2026-07-28` `subscriptions/listen` replaces
`resources/subscribe` and makes the listen request carry its own subscriptions,
which would make the stored set unnecessary.

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
presence. The NOTIFY + per-replica TTL view reuses the substrate with
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

**Alternative.** Fan the state over NOTIFY as presence and the resource bus do, or keep an
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

### Request rejections are problems; an unreadable request is a 400

**Decision.** Handlers take their input through `crate::extract` (`ApiPath`,
`ApiQuery`, `ApiJson`, `ApiBytes`, `ApiText`), never axum's extractors
directly, so a rejection is an RFC 9457 problem. A path parameter, query
string or JSON body that does not deserialize is a 400, whether it fails to
parse or parses to the wrong shape; a body not sent as JSON is a 415; a body
over `MAIDAN_MAX_BODY_BYTES` is a 413. An unknown route is a 404 and a wrong
method a 405 (with `Allow`), from router fallbacks. SCIM and A2A declare their
own wrappers with `wrap_extractor!` and answer in their envelopes (a SCIM
error; a JSON-RPC error with a null id, or the REST binding's error body). The
spec's 400/413/415 are derived from what each operation declares it takes
(`ExtractorResponses`).

**Alternative.** axum's 422 for a body of the wrong shape. A response-mapping
layer that rewrites axum's text rejections after the fact.

**Why this:** the API already answered every malformed body with 400
(`ApiJson` mapped axum's 422 to it), and clients and the spec relied on that;
one status for "the request cannot be read" is simpler to handle than two. A
rewriting layer would have to parse axum's text back into a status and
detail, and would miss nothing only by accident; the extractors fail with the
right problem at the source. Since axum 0.8 (#1081) the compiler keeps a raw
extractor out: routes register through `crate::routing`, whose `get`/`post`/
`put`/`patch`/`delete` accept a handler only when every argument implements
`Checked` (the wrappers, and extractors that cannot fail on client input), and
`clippy.toml` disallows axum's own routing functions. This replaced a source
scan that matched handler signatures by text.

**To revisit:** if axum grows a way to set a rejection type per router.

### A2A v1.0 as the official TCK reads it

**Decision.** The A2A endpoint follows A2A v1.0.0 as the official TCK
checks it, over JSON-RPC (`/a2a/v1/rpc`) and HTTP+JSON (`/a2a/v1/...`):

- **Version.** A request names its version in the `A2A-Version` header (or
  the `A2A-Version` query parameter). A missing version means 0.3, which is
  refused with `VersionNotSupportedError`; only `1.0` is served.
- **Contexts are threads.** A `contextId` Maidan has seen maps to its
  thread; a thread id from the caller's workspace is that thread; any other
  string opens a thread in the workspace's public `a2a` channel. A task
  continues only in its own context.
- **The author is the caller.** `SendMessage` posts as the token's member,
  never a `metadata` field, and a bypass token is refused: the message must
  have a real author.
- **Tasks carry no words.** The task row stores ids, state and timestamps;
  history is rendered from the stored message, so a tombstoned or shredded
  message is gone from the task too. The migration strips old
  `status.message` copies.
- **Push secrets are sealed.** A push config's `token` and credentials are
  encrypted with the at-rest key (`FEDERATION_ENCRYPTION_KEY`), never
  returned, and a server without the key refuses them.
- **Errors** are the spec's: JSON-RPC codes with a `google.rpc.ErrorInfo`,
  and AIP-193 bodies with the matching HTTP status on the REST binding,
  which answers `application/json`. A task the caller cannot read is
  `TaskNotFoundError`, never a hint that it exists.
- **Listing** pages by an opaque `(updated_at, id)` cursor. Thread access is
  decided in the store query, so a page is `pageSize` tasks the caller can
  read unless it is the last, `totalSize` counts only those, and what a call
  costs does not depend on what the caller cannot read.
  `statusTimestampAfter` is inclusive, as the proto says.

**Alternative.** Keep the Maidan subset (`metadata.maidan.threadId` and
`authorId`, `application/a2a+json`, no version check) and document the
differences.

**Why this:** a generic A2A client could not talk to the subset, and
trusting a body `authorId` let one member post as another.

**To revisit:** a new A2A major version, or when the gRPC binding (task
read/cancel/list only) grows to the full surface and joins the TCK run.

## Security

### The search tap resumes; the verifier verifies (`v402.2.0`)

**Decision.** The search tap persists a resume cursor and walks forward from it.
Whole-chain integrity moves to an explicit, schedulable check — `GET
/workspaces/:wid/events/verify` — rather than being a side effect of process
restarts.

**Why the old behaviour could not stay.** `backfill_search` walked from event id
0 unconditionally: on process start, on every resubscribe, and on every
`Lagged`. On Postgres the handler is `BatchingEmbeddingHandler`, so each walk
re-embedded the entire history. Worse, it livelocked — the bus is not drained
*during* a backfill, so a busy instance overflows the broadcast while walking,
gets `Lagged` on the first poll, and walks from 0 again. On a large log it never
converges.

**Why the "cursor vs re-walk" framing is wrong.** It treats one component as
doing one job. The tap does two: **projection** (keep the search index current)
and **verification** (notice a tampered log). Projection wants to be
incremental. Verification wants a full walk. Today projection paid
verification's cost, and verification's failure mode — stop everything — was
applied to projection.

**What is actually given up.** A tamper *behind* the cursor is no longer noticed
by the tap. That is a real reduction and it should not be softened. But the tap
only ever noticed such a tamper when the process happened to restart, which is
an accident of implementation rather than a control anyone could rely on,
schedule, or alert from. `verify_chain` streams rather than materializing, so a periodic
full verify is affordable; that is a control you can actually operate.

**What is not given up.** Every event the tap projects is still chain-verified
against its workspace's previous link. On a resume the predecessor is
re-derived from the log — *not* stored alongside the cursor, because a second
copy of the hashes could drift from the log it is supposed to attest. Without
that seeding, `verify_link` skips the `prev_hash` comparison entirely for a
mid-chain row (`previous = None`, `from_genesis = false`), so an unseeded resume
would accept a row whose predecessor had been deleted or reordered —
verification that looks like it ran and did not.

**Cursor lifecycle.** Not persisted while any workspace is faulted: the
high-water covers rows a faulted tenant's index does not have, and saving it
would make the gap permanent. Cleared entirely on a whole-tap fault, because
resuming past a detected break preserves exactly the divergence that was
detected. Monotonic, so a slower replica cannot drag it backwards.

**Not `maidan_delivery_cursor`.** Retention's floor is `min_delivery_cursor`, so
registering the tap there would let a stuck tap block log pruning forever. A
separate table keeps the failure isolated: a stuck tap hits `CursorTooOld` and
rebuilds, which is the designed path.

**Trade accepted.** The tap verifies what it projects. The chain is the
verifier's job, and it is now a job someone can schedule rather than a thing
that happened when a process bounced.

### Revoking a token revokes everything derived from it (`v401.3.0`)

**Decision.** `maidan_api_tokens` gains a `parent_token_id`, attenuation records
it, and `revoke_api_token` revokes the whole subtree — transitively, not one
level.

**Why cascade rather than mark.** An earlier decision established the principle: *a
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
been a hundred mechanical edits serving one caller — the ripple that adding
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
member who has ever held a thread. Both governance gates (required reviews,
the land gate) ask *"has this reviewer ever worked this thread?"* rather
than *"is this reviewer the current assignee?"*.

**Why the live column could not stay the input.** Both gates tested
`thread.assignee_id`, and releasing a claim sets it to `NULL` — so the exclusion
became vacuous at exactly the moment someone wanted it to be. Do the work,
release the claim, approve your own work as a qualifying third party. The gate
still ran; it just had nothing left to compare against.

**Why not the event log.** Assignment history *is* recorded there, and reading
it would need no new table. But retention prunes the event log, and
a security control cannot depend on evidence that ages out. A gate that weakens
after ninety days is a gate with a calendar.

**Why not a `last_worked_by` column.** It is smaller and it is wrong: it
remembers only the most recent holder. A bearer token is act-as-any by design
(the orchestrator model), so an agent could claim *as* another
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


### Approvals may be borrowed, never self-approved (Cluster 411.10)

**Decision (maintainer, 2026-09-23).** An approval — a review, a land-gate pass,
an approval-gate answer — may be made with a borrowed token: a delegate holding
a grant from a reviewer approves *for* that reviewer. It never counts when the
member actually acting owns, holds or worked what it approves, and nobody
accepts an approval gate they requested, whichever identity they asked or
answer under.

**Why this needed saying.** A delegated token *is* its subject, so every
separation-of-duties check that compared the subject could be laundered: claim a
thread as the worker, then approve it as the reviewer. The checks were correct
about members and blind to delegates.

**How.** The actor is recorded beside the subject on each attestation
(`maidan_thread_reviews.actor_id`, `maidan_thread_land_gate.recorded_actor_id`,
`maidan_approval_gates.requested_actor_id` / `resolved_actor_id`) and on the
worker ledger — a delegate that claims for a member has worked the thread too.
The store reads it from the request's attribution scope, so no call site can
forget to pass it. Every exclusion then tests the actor as well as the subject,
on all four surfaces the release-laundering fix (401.2) patched.

**Rejected.** *Own credential only* — refusing every borrowed approval — is
stricter and simpler, but makes delegation useless for the review automation it
exists for. *Borrowable, just recorded* leaves the laundering open and relies on
someone reading the audit trail afterwards.

**Also closed.** Approval gates had no self-approval rule at all, delegated or
not: any member with `workspace:write` could accept its own request. Declining
or cancelling your own request is still allowed; it approves nothing.

### Decisions keep their history; approval answers are final

**Decision.** Every review verdict and every land-gate verdict is appended to
a history table (`maidan_thread_review_verdicts`,
`maidan_thread_land_gate_verdicts`) in the same transaction as the write.
The current rows stay as they are: each reviewer has one current review and each
thread has one gate pointer, and the close-gate reads those. The history is read with
`GET /threads/{id}/reviews/history` and `GET /threads/{id}/land-gate/history`
(MCP `list_review_history`, `list_land_gate_history`), oldest first.

**Why.** The attribution row (411.9) records that a decision changed and who
changed it, not what it was. A re-submitted review or a new gate pointer
overwrote the earlier value, so an owner auditing why work landed could see
only the last verdict. Dismissing approvals on a send-back and clearing the
gate change the current rows only. The history keeps what was decided before.

**Approval-gate answers need no history.** Resolving a gate is a
compare-and-set from `pending`. The first answer is final, and a second answer
changes nothing (pinned in `approval_gates.rs`). A gate's row already is its
history: requested, then answered once.

**Rejected.** *Versioning the current rows* (a `superseded_at` column on
`maidan_thread_reviews`) would make every close-gate and status query filter on
it, and a missed filter would count a stale approval. *Deriving history from the
event log* does not work: reviews and gate pointers do not all emit events, and
reconstructing from events would tie an audit read to event retention. A history
table is append-only, and nothing that decides a land reads it.

**Backfill.** Migration 0121 seeds the history with one verdict per existing
current row, stamped with its last update time. Values overwritten before
0121 were not recorded and cannot be recovered.

### A workspace handle is a display label, not an address (`v398.7.0`)

**Decision.** Maidan will **not** resolve a handle to a workspace. There is no
`GET /rooms/:handle`, and the store's reverse lookup (`workspace_id_for_handle`,
shipped unused) is removed. A handle is a renameable, unique,
human-readable label shown on the room card — nothing addresses by it.

**Context.** The handle table shipped with `PUT`/`GET
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
- **`maidan://` URIs deliberately use the UUID.** That was chosen so a
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

**Context.** The ABI landed (`crates/maidan-types/src/wasi.rs`) and
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
filters, and the federation/A2A ingress (the channel-RBAC arc,
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
  an act-as-any orchestrator; a single `current_workspace` GUC
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

### Load shedding is a global in-flight ceiling outside authentication

**Decision.** One semaphore bounds the HTTP requests in flight
(`MAIDAN_MAX_CONCURRENT_REQUESTS`, default 1024). A request that finds no
free permit gets a `503` problem with `Retry-After: 1` at once; nothing
queues. The layer sits outside the rate limiter and authentication, so a
refused request costs no Redis round-trip, token lookup or pooled
connection. `/health*` and `/metrics` are exempt, the same paths the rate
limiter exempts. A permit is held until the response head is ready, so a
long-poll wait holds one; streamed bodies and WebSockets do not, and have
their own ceilings. A handler panic answers a `500` problem
(`CatchPanicLayer`) inside the metrics and request-id layers, so it is
counted and carries the id its log line has.

**Alternative.** tower's `ConcurrencyLimit` + `LoadShed` (`Router::layer`
gives each route its own semaphore, and exempting paths needs a wrapper);
a queue with a timeout (`ConcurrencyLimit` alone), which turns overload
into latency until clients time out and retry into the pile; a per-tenant
ceiling, which is the per-workspace rate limit's job.

**Why this:** failing fast keeps latency flat for the requests that are
admitted, and a `503` before any work is safe to retry, even for a write.
Probes stay answered so an overloaded replica is not also restarted.

**To revisit:** if long-poll waits come to dominate the ceiling, move them
to their own permit pool.

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

### Crypto-shredding of message content

**Decision.** A message's words (body, metadata, content blocks) are
sealed before its event is hashed, and withdrawing the message destroys
the key.

- **Subject: one message.** Withdraw and purge act on one message, so
  one key per message erases exactly what was withdrawn. Its posted and
  edited events share the key. A replicated message's subject is
  `uuid5(origin peer, message id)`, so only the origin's tombstone can
  shred it.
- **Crypto.** XChaCha20-Poly1305 (`chacha20poly1305`, RustCrypto) with a
  random 24-byte nonce. The event keeps `message.body = ""`, drops
  metadata and content, and carries `sealed {alg, nonce, ciphertext}`.
  The hash chain and signatures cover the ciphertext, so verification
  needs no key and passes after a shred.
- **Keys.** `maidan_content_keys` holds one 256-bit key per subject,
  wrapped by the key-encryption key (`MAIDAN_CONTENT_KEK`, same AEAD,
  subject id as associated data) and tagged with the KEK's fingerprint.
  Rotation: new primary, old one in `MAIDAN_CONTENT_KEK_PREVIOUS`; the
  server rewraps at startup. An unknown KEK is an error, never read as
  shredded. Without a KEK the server refuses to start (see *The content
  KEK fails closed*).
- **Shred.** Appending `message.tombstoned` sets the key row's wrapped
  key to NULL in the same transaction, deletes the subject's pending
  webhook deliveries, egress outbox rows and notification mail, and
  blanks the message row's metadata. A later event for the subject is
  sealed under a throwaway key. Workspace purge deletes all its keys.
- **Mail.** A mail row carries the key of the event it notifies about
  (`content_key_id`, cascading). Mail bodies hold no words, only the
  notification kind and event number, but a notice that a withdrawn
  message exists is still a trace of it, so a shred deletes the mail and
  an enqueue for a shredded subject is skipped (Postgres takes the key
  row `FOR SHARE`; SQLite checks in the insert).
- **Verify.** `maidan verify-shredding` lists copies of withdrawn words
  outside the sealed log (message rows, edits, unsealed payloads,
  webhook, egress and mail rows, search entries, every embedding table).
  It is a read-only query per table, so it ships as a CLI rather than an
  API.
- **Reads.** Members get events opened; a shredded event reads as an
  empty body with its `sealed` block. Admin catch-up, peer catch-up and
  federation envelopes carry the ciphertext plus the key only while it
  is live (`content_key`); a peer that ingests the tombstone shreds its
  copy. Exports, snapshots, search and embeddings never see shredded
  words.
- **Artifacts.** `DELETE /artifacts/:sha` drops one workspace's
  reference and its share-ticket grants. The row and the bytes go only
  with the last reference (Postgres locks the row). Refused under a
  legal hold.
- **Blob reap.** Deleting bytes races an upload of the same sha: the
  upload writes the bytes (already there), then its row, while the erase
  deletes them. `reap_artifact_blob` checks for a row and writes a lease
  on the sha (`maidan_artifact_reaps`) in one short transaction, under the
  sha's lock (Postgres advisory xact lock; SQLite `BEGIN IMMEDIATE`).
  It then deletes the bytes outside any transaction and drops the lease.
  Every artifact upsert takes the same lock and waits while a live lease
  holds its sha. An upsert therefore lands before the check (the reap
  keeps the bytes) or after the delete, and then the uploader writes the
  bytes back (`restore_if_reaped`). Workspace purge reaps the same way.
  The delete is bounded by `BLOB_DELETE_TIMEOUT` (30 s); one that overruns
  is reported as failed and its lease, twice that long, is left to lapse,
  since it may still land. A lease left by a reaper that crashed lapses
  the same way. Before, the reap held its transaction for the whole blob
  delete: on SQLite that blocked every writer, and on Postgres it held a
  pooled connection idle in a transaction, which
  `idle_in_transaction_session_timeout` could end with the delete still
  running.

**Alternative.** Per-author or per-workspace keys: coarser, so a single
withdrawal cannot be erased without re-encrypting everything else.
Rewriting history instead: breaks the hash chain and every signature
peers hold.

**Why this:** the log stays append-only and verifiable while the words
become unrecoverable on every tier that holds only the database.

**Limits.** A backup taken before a shred, together with the KEK,
recovers the words (keep KEKs out of data backups). A peer that already
opened the words keeps them unless it ingests the tombstone. Under a
legal hold the preserved copy keeps the words by design. A mail send
already handed to SMTP cannot be recalled.

### The content KEK fails closed

**Decision.** The server and `maidan init` refuse to start without
`MAIDAN_CONTENT_KEK`. The built-in development key is used only with an
explicit `MAIDAN_ALLOW_INSECURE_DEV_KEK=1`, which is refused together
with `MAIDAN_ENV=production`. The compose files and dev scripts set the
flag; the Helm chart fails to render without `contentKek` or an
`existingSecret`, and the k8s base reads the key from `maidan-secrets`.

**Alternative.** Fall back to the dev key unless `MAIDAN_ENV=production`
(the previous rule). A deployment that forgot `MAIDAN_ENV` sealed real
words under a key anyone can read in the source, and nothing said so
but a log line.

**Why this:** it matches `AUTH_DISABLED`, which needs
`MAIDAN_ALLOW_INSECURE_NO_AUTH`: the insecure mode is named where it is
turned on, and a missing variable stops the process instead of weakening
it. The library's `Store::new` keeps the dev keyring for tests; the
binaries always build the keyring from the environment.

**To revisit:** never.

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
(see [Open Work](Open%20Work.md)). `InMemoryBus` stays full-envelope.

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

**Decision.** Live subscription stays the low-latency optimistic
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

**Superseded.** `macos-13` runners could sit queued for hours and hold the
whole tag release open, so the Intel target moved out of `release.yml` into
the manual `release-darwin-x86.yml` (`workflow_dispatch` with a tag). A tag
release builds `aarch64-apple-darwin` on `macos-latest` only.

### The OpenAPI lint is a unit test; Redocly is a local script

**Decision.** The `openapi::lint` unit tests in `maidan-server` assert what
Redocly's recommended ruleset checks on the served document: a summary on
every operation, a 4xx on every operation that can return one, each client
error an RFC 9457 problem, 401 and 429 wherever the auth and rate-limit
middleware can answer them, unique operation ids, OpenAPI 3.1 with servers and
no unused components. `scripts/openapi-lint.sh` runs Redocly (pinned) against
a live server. The middleware responses are added by one utoipa `Modify`
(`MiddlewareResponses`) from the security requirement and
`rate_limit::exempt_path`, not by hand on 300 path stubs. utoipa 5 emits
OpenAPI 3.1, which writes a nullable reference as `oneOf [null, $ref]`.

**Alternative.** A CI job running `@redocly/cli`. Stay on utoipa 4 and
disable `nullable-type-sibling`.

**Why this:** the unit test runs in the required `unit tests` job with no
Node toolchain and fails with the operation that regressed. A new CI job
would be report-only until branch protection names it. The 3.0 output
could not be made valid without a rule exception; 3.1 is.

**To revisit:** if Redocly's recommended set gains a rule the tests do not
mirror, or when utoipa 6 is adopted.

### CI tests run under nextest; only quarantined tests retry

**Decision.** `unit tests`, `integration (testcontainers)` and
`coverage (llvm-cov)` all run `cargo nextest run --profile ci`, configured in
`.config/nextest.toml`. The default profile reports a test that runs past 60
seconds and kills it at 3 minutes, and never retries. The `ci` profile runs
every test, writes a JUnit report that the two required test jobs upload, and
gives two retries to a named quarantine list and nothing else. A test goes on
that list only with a failed `main` run to point at and a row in Open Work,
and comes off when it is fixed. A retry that passes shows as `FLAKY` in the
log, the summary and the JUnit report. There are no test groups: each test
runs in its own process, so tests that set environment variables or install a
global subscriber cannot interfere, and the Postgres suites (one container per
test) run at the default concurrency without contention.

**Alternative.** `cargo test` for the unit job; a blanket `retries = 2` in CI;
a serial group for environment-touching tests.

**Why this:** one runner and one config for every test job, with a hang
failing its own test instead of running into the job's timeout. A blanket retry
turns every intermittent bug into a silent pass. Per-process isolation makes a
serial group unnecessary. The required check names are unchanged.

**To revisit:** if a test group becomes necessary (a shared external resource),
or if the quarantine list grows past a handful.

### Coverage floors are per crate and advisory

**Decision.** `.config/coverage-floors.toml` sets a line-coverage floor for
the workspace and for every crate, each just under the crate's measured
coverage (one point off the measurement, rounded down to the half point).
`scripts/coverage-floors.py` fails the `coverage (llvm-cov)` job when any of
them is missed, when a crate has no floor, or when a floor names a crate that
is gone. The job stays out of the required checks. It replaces the single
`COVERAGE_MIN_LINES` floor of 40% against a measured 86.7%.

**Alternative.** One workspace percentage; floors enforced by making coverage
a required check.

**Why this:** a workspace number is dominated by the server and the store, so
a crate like `maidan-auth` (1,300 lines) or `maidan-fsm` (100) could lose a
third of its tests without moving it; per-crate floors put the gate where the
risk is and leave thin crates visible as gaps instead of padding them out. The
job takes about 14 minutes against under 4 for `unit tests`, and it measures the same
suite the required `integration (testcontainers)` job already gates, so
making it required would slow every merge to catch coverage drops that a
red advisory job already shows. Open Work does not call for it.

**To revisit:** if a floor is missed on `main` without anyone noticing, make
the job required.

### Testcontainers tests skip only when no Docker daemon answers

**Decision.** A testcontainers test whose container fails to start calls
`maidan_store::test_support::docker::skip_start_failure(err)`. It returns,
so the test can skip, only when the daemon testcontainers would use does not
answer a ping; with a daemon up it panics with the start error. Once the
container is up, host, port, connect and migrate errors `expect` instead of
returning `None`.

**Alternative.** Skip on any start error, the old pattern; or never skip and
require Docker for `cargo test`.

**Why this:** skipping on any start error let `s3_roundtrip` and
`s3_multipart` pass in CI for as long as `minio/minio` could not be pulled,
with `s3.rs` at 0% coverage. A pull failure, a missing tag or a readiness
message that never comes looks exactly like "no Docker" under that pattern,
and a `.ok()?` after the start hid a migration that failed the same way.
Contributors without Docker can still run the suite.

**To revisit:** if CI ever runs a test job without Docker, which would turn
the integration tests back into skips; the integration job's test count
would show it.

### The A2A TCK is a non-required CI job that fails on regressions

**Decision.** The `a2a tck` job runs `scripts/a2a-tck.sh`: the official
A2A TCK at a pinned commit (tag `1.0.0.alpha2` plus harness fixes) against
a source-built, auth-enabled server over JSON-RPC and HTTP+JSON. It fails
on any TCK failure, on an exclusion that no longer matches a test, and when
fewer tests pass than the recorded floor (a failed set-up request makes the
TCK skip, so a regression can surface as a skip). It is not a required
check and has no `continue-on-error`: a red run is visible without blocking
a merge. It replaces the report-only `a2a interop` job, whose client
(`examples/a2a_interop.py`) now runs first in the same job.
`scripts/a2a-tck/exclusions.txt` holds the tests Maidan does not run, each
with its reason: the artifact and direct-Message tests need the TCK's
scripted reference agent, and `CORE-SEND-003` has no expected error in the
TCK, so its generic runner demands success from a request that must fail.

**Alternative.** A required check; or report-only (`continue-on-error`), as
`a2a interop` was.

**Why this:** Open Work does not say required. The TCK is an alpha fetched
from GitHub at run time, so a network or upstream problem would block
merges, and the A2A behaviour it covers is also pinned by the required Rust
e2e tests. Report-only would hide a regression behind a green tick.

**To revisit:** when the TCK cuts a stable release: pin it, raise the floor,
and consider making the job required.

### Loom models sit behind a `loom` cargo feature

**Decision.** `maidan-bus` and `maidan-server` have a `loom` feature that,
in the crate's own test build, swaps its locks for loom's and compiles only
the loom models; every other build keeps std's locks. The
non-required `loom` CI job runs them in release mode. The models cover the
sharded bus (subscribe/publish/prune) and the presence hub (reconnects,
racing status changes, a sweep racing a heartbeat).

**Alternative.** `RUSTFLAGS="--cfg loom"`, the convention in loom's docs.

**Why this:** tokio reads `cfg(loom)` too and then expects loom as its own
dev-dependency, so the whole build breaks; `RUSTFLAGS` would also rebuild
every dependency. Claims, the outbox and WIP checks are SQL transactions,
which loom cannot model; `claim_state_machine` covers them. Code where each
operation runs under one mutex (MCP resource subscriptions) or holds no
shared state (the event-stream watermark, counters) has no interleaving to
check.

**To revisit:** when another in-process structure shares state across
tasks without one lock around each operation.

### Presence changes are announced under the hub lock

**Decision.** The presence hub decides a change, sends the local frame and
queues the cross-replica event while it holds its lock. One publisher task
sends the queue in order, giving each publish 5 s; the queue is bounded
(16,384) and drops changes, with one warning, while full. A heartbeat takes one slot: the publisher reads
the local members when it reaches it. A replica ignores its own events from
the notifier, and reports a member connected to it from local state. It
keeps each other replica's word on a member separately; the member is online
if any replica says online, and one replica's `offline` removes only its own
entry.

**Alternative.** Announce after the lock is released, with one spawned task
per publish (the old design).

**Why this:** the loom models showed a reconnect's `online` overtaken by the
old connection's `offline`, so subscribers saw a connected member offline;
racing status changes and a sweep racing a heartbeat ended wrong the same
way. Sending to a broadcast channel under the lock does not block.

**To revisit:** if the lock shows up in presence latency.

### TLA+ specs, checked by TLC in CI, each with a config it must fail

**Decision.** `specs/tla` holds two specs: `Claim` (claim_next, claim by
id, assign, unassign, freeze, renew, acknowledge, release, lease lapse and
the reaper, with fencing tokens) and `EventLog` (the hash chain and crypto-shredding on
an origin and a peer, over a network that drops, duplicates and reorders).
The non-required `tla` job runs `scripts/tla.sh`: TLC 1.7.4, pinned by
SHA-256, checks each spec's config, then a config with one mechanism off
(the old deadline handling; the peer's chain check), where TLC must report
the named invariant violated.

**Alternative.** Specs without CI, or only passing configs.

**Why this:** a spec nobody runs drifts from the code. A passing check says
nothing if the invariant is too weak to fail; the failing config shows it
catches the bug it names.

**To revisit:** when the claim or sealing code changes shape: change the
spec in the same PR.

### osv-scanner covers the lockfiles cargo-deny does not; SBOMs are image attestations

**Decision.** The non-required `osv scan` CI job runs `scripts/osv-scan.sh`:
osv-scanner 2.6.0, pinned by SHA-256, over every tracked lockfile except the
root `Cargo.lock` (`fuzz/Cargo.lock`, `ui-tests/package-lock.json`,
`sdk/go/go.mod`) and a fresh resolution of `sdk/rust`, whose `Cargo.lock` is
ignored because the crate's users resolve their own. The TypeScript and Python
SDKs have no runtime dependencies; the script fails if one gains a dependency
without a lockfile. Accepted advisories, each with its reason, are in
`.config/osv-scanner.toml`. It fails on anything else and has no
`continue-on-error`. The release attests CycloneDX SBOMs by digest
(`cosign attest --type cyclonedx`, keyless, the same identity as
`cosign sign`): cargo-cyclonedx for the server and CLI, attested to each
image's index, and trivy for Postgres, one SBOM per platform attested to that
platform's manifest, because a trivy scan reads one platform's packages. The
same files are published and blob-signed beside the tarballs.

**Alternative.** osv-scanner over the whole repository, root lockfile
included; a required check; SBOMs only as release assets.

**Why this:** cargo-deny reads the root lockfile against RustSec with the
real dependency graph, and `deny.toml` holds its accepted advisories. OSV
reads lockfiles without the graph, so over the root it reports crates that
are locked but never built (`quinn-proto`), and it would need a second copy of
those reasons. A new advisory can turn the job red with no change here, so a
required check would block unrelated merges. An SBOM that is only a release
asset is not bound to the image a user pulls; an attestation on the digest
is, and `cosign verify-attestation` checks it against the release workflow.

**To revisit:** OSV also carries GitHub advisories that RustSec does not
(`cmov`, `opentelemetry_sdk` and `serde_with` in the root lockfile, as of
2026-09-29). If those keep appearing, scan the root lockfile too, with its
accepted advisories mirrored from `deny.toml`.

### Every write that changes a thread's holder sets its lease deadline

**Decision.** `claim_next_thread` writes the lease deadline (or none);
`claim_thread`, `assign_thread`, `unassign_thread`, `release_claim` and the
budget release clear it.

**Alternative.** Leave the deadline alone outside `claim_next_thread` and
`renew_claim` (the old behaviour).

**Why this:** the claim spec found that a holder that never took a lease
could inherit an earlier holder's deadline (after a release, or an assign
over a live lease). Once it passed, `claim_next_thread` took the thread and
reported `ClaimExpired` for a holder with no lease. `claim_deadline` pins
the fix on both backends.

**To revisit:** if assign should hand over a lease; it would then take a
lease length, not inherit a deadline.

### A seeded simulation of the NOTIFY floor, not madsim

**Decision.** The listener's floor logic (`notify_floor`) reads the log
through an `EventLog` trait. A unit test runs it on three replicas against a
model log for 400 seeds (splitmix64, one thread, no clock): transactions
commit out of order or roll back, NOTIFYs are lost, replicas reconnect and
reads fail. It checks that only committed events are delivered and that every
committed event arrives, except the one gap the floor cannot see (a late
commit at or below the mark whose NOTIFY was lost). `MAIDAN_SIM_SEED`
replays one seed with its full trace.

**Alternative.** madsim, which runs the real crates under a simulated
runtime.

**Why this:** madsim needs `--cfg madsim` across the whole build, and sqlx,
reqwest, tonic and hyper do their own I/O, so the server would not run under
it without replacing its drivers. The floor is where replicas can lose
events, and it is pure logic once the log is behind a trait. The loom models
cover the presence hub's interleavings.

**To revisit:** if more of the replication path (federation ingest, the
outbox relay) is moved behind traits, simulate it the same way.

### The NOTIFY floor only moves past delivered events

**Decision.** After a pointer, the mark moves to the pointer's id only if the
gap below it was fully back-filled and the pointer's event was delivered;
otherwise it stays at the last id delivered in order. The starting mark is
the log head read at connect, after LISTEN; if that read fails, the connect
fails rather than guess a mark.
A single row that cannot be decoded (a bad kind, a content key that will
not unwrap) is skipped and counted as a failed hydrate, not a store error:
the back-fill reads each row's result beside its id, so one such row cannot
hold the mark below it forever. That row is the one exception to "delivered":
the mark can move past its id without publishing it.

**Alternative.** Move the mark to the pointer's id regardless (the old
behaviour).

**Why this:** the simulation found that a store error during the back-fill,
or a failed read of the pointer's event, moved the mark past ids never
delivered, and no later NOTIFY or reconnect drained them. Staying low can
deliver an event twice, which the at-least-once contract allows.

**To revisit:** never; this is the floor's contract.

### A lapsed lease is reaped on a timer, and every claim_next claim is leased

**Decision.** A claim reaper runs on every replica
(`MAIDAN_CLAIM_REAP_TICK_SECS`, 5 s, on by default). Each tick,
`Store::reap_expired_claims` frees open threads whose lease lapsed, oldest
deadline first, in batches of 100 up to 1000 a tick, and appends
`ClaimExpired` for the holder in the same transaction. Postgres picks the
batch with `FOR UPDATE SKIP LOCKED`, so replicas split the work and never
report a lease twice; SQLite guards the update on the same holder and a
still-lapsed deadline. `claim_next_thread` still takes a lease that lapsed
between ticks. A thread in review is not reaped. `claim_next_thread` claims
always carry a lease: with no `lease_secs`, the server default
(`MAIDAN_CLAIM_DEFAULT_LEASE_SECS`, 600 s); a named lease, and every
renewal, must be 1 s to 7 days. REST and MCP resolve it through one
`ClaimLeasePolicy`. `assign_thread` and `claim_thread` stay unleased.

**Alternative.** Keep lazy reclaim and fire `ClaimExpired` from the next
`claim_next` (a dead holder on an idle channel is never reported); a
Postgres-only `LISTEN`/timer per lease (no SQLite story, one timer per
claim); keep `lease_secs` optional (a forgotten lease is a claim that never
comes back, the common way work got stuck).

**Why this:** the room should tell a supervisor an agent died when its lease
runs out, not when someone else happens to look for work. A sweep of a
partial index on `assignment_expires_at` is cheap at a 5 s tick, and the
spec's `Reap` action shows it reports only lapsed leases and fences the
dead holder's token.

**To revisit:** if assign or claim by id should take a lease, add a
`lease_secs` to them rather than a default; an explicit handoff is an
operator's decision to hold the thread.

### An unacknowledged claim is reported, not reclaimed

**Decision.** The claim reaper's tick also calls
`Store::report_unacknowledged_claims`: a leased claim on an open thread,
still live, taken more than `MAIDAN_CLAIM_ACK_TIMEOUT_SECS` (120 s) ago
and not acknowledged gets one `ClaimUnacknowledged` naming the holder and
when it claimed. `claimed_at` is written with every new fencing token and
cleared with it; `unacknowledged_lease_id` records the token last reported,
so each claim is reported once and a new claim has its own window. The
claim is not changed. Claims with no lease are not reported. The event is
not federatable and counts as stuck work in notifications and the manager
digest.

**Alternative.** Release a claim that is not acknowledged in time (it would
take work from a slow agent that is fine, and it duplicates the lease);
report on every tick (a flood for one stuck claim); leave it to polling the
occupancy view (the Cluster 351 state, which nobody watches).

**Why this:** "claimed but never started" is the one stuck case the lease
cannot see until it lapses, which may be many minutes. A push lets a
supervisor act early, and the lease stays the single mechanism that takes
work back. The claim spec is unchanged: the report writes no claim state.

**To revisit:** if operators want a per-channel or per-agent window rather
than one server setting.

## Workflow

### Admin-merge instead of local-first push

**Decision.** PRs are squash-merged via `gh pr merge --squash --admin
--delete-branch`. Branch protection on `main` requires the 8 CI checks,
but with `enforce_admins` off `--admin` bypasses them as well as the
required review. So green-before-merge is a rule the maintainer keeps,
not one GitHub enforces: an admin-merge over a red required check needs
a stated reason. About 30 PRs were merged over a red `docker compose
smoke` (#973→#1005) before the egress-guard regression behind it was
found, which is the cost of forgetting that.

(Until 2026-09 this entry said `--admin` did *not* bypass the checks.
It did, the whole time.)

**Original direction (deferred).** Local-first push: nothing gets
pushed until `make ci` passes locally; remote `main` stays
buildable; no admin-merge.

**Why the reversal:** the user (sole maintainer) found local-first
slowed iteration without adding safety since they were the only
reviewer anyway. The CI-required-checks discipline replaces the
local-first discipline. Local CI is still encouraged but not
load-bearing.

**Reaffirmed 2026-10-01.** The maintainer kept admin squash with no review
requirement while one person maintains the repo. The merge loop adds what
GitHub does not enforce: all eight required checks passed on the PR's exact
head commit (`--match-head-commit`), every CodeRabbit comment answered, and a
build of the PR merged onto current `main` with the static contracts.

**To revisit:** when a second human reviewer joins the project. At
that point, restore PR-review enforcement and drop the `--admin`
flag.

### Branches need not be up to date to merge (F-43 stays off)

**Decision.** Branch protection's `strict` setting stays off. A PR whose base
is behind `main` can merge once its own checks pass. The merge loop builds the
PR merged onto current `main` and runs the static contracts first, and
`main`'s own CI runs after every merge.

**Alternative.** Turn `strict` on, so every PR reruns CI after each merge.

**Why this:** with about twenty runners and five to ten PRs open at once,
`strict` turns each merge into a full CI rerun for every other PR, and most
of what it would catch (a counter two PRs bump, a contract a new route must
satisfy) the pre-merge build catches in minutes. What it misses is a break
that only the full test suite on the merged tree shows, which `main`'s CI
then reports.

**To revisit:** if a merge breaks `main` in a way the pre-merge build could
not see, or when CI capacity makes the reruns cheap.

### Squash-merge only; PR body becomes the commit body

**Decision.** Merge commits and rebase are disabled at the repo
settings level. The PR title becomes the squash commit title; the PR
body (including the **mandatory** PR-level retro section) becomes
the commit body.

The repository's squash title and message are set to `PR_TITLE` and
`PR_BODY` (since 2026-10-01; until then the message was `COMMIT_MESSAGES`,
so a squash commit carried the PR's commit messages and the retro lived only
on the PR).

**Why this:** every commit on `main` carries its own retro inline.
`git log` is searchable. Cluster-level retros aggregate the per-PR
retros.

**To revisit:** never. This is load-bearing for the retro discipline.

### CodeRabbit reviews every PR; its comments are addressed, not required

**Decision.** CodeRabbit reviews every non-draft PR to `main` and each
new push, with the settings in `.coderabbit.yaml`. A PR is squash-merged
when the 8 required checks are green and every CodeRabbit comment is
addressed: fixed, or answered with the reason it does not apply.
CodeRabbit is not a required check and does not approve or block PRs.
Its summary goes in the walkthrough comment, not the PR body, because
the body becomes the squash commit.

**Alternative.** Make its review a required check, or run without an
automated reviewer.

**Why this:** a solo maintainer has no second reader, and an AI review
catches things CI does not. A required check would let a wrong comment
block a merge; addressing each comment keeps the reader without that cost.

**To revisit:** when a second human reviewer joins, or if its comments
are mostly noise.

### Annotated unsigned tags acceptable pre-1.0

**Decision.** Cluster tags are annotated (`git tag -a`) but not
signed. The user has not configured GPG/SSH signing as of `v0.1.0`.

**Alternative.** Block tagging until a key exists.

**Why this:** signing is a separate, mostly-one-time setup task.
Don't gate every release tag on it. Future tags can be re-issued
signed if needed.

**To revisit:** when a key exists. (No key exists yet; tags through
`v412.0.0` are annotated and unsigned, and "pre-1.0" in the title means
before a signing key, not before `v1.0.0`.)

### Semver-stable API from v1.0.0

**Decision.** From `v1.0.0`, HTTP route shapes and MCP tool/resource
names are treated as stable public API. Breaking changes require a
major version (`v2.0.0`). Pre-1.0 clusters could rename and delete freely.

**Why this:** agents and operators integrate against HTTP and MCP;
predictability matters once federation and UI exist.

**To revisit:** only via a deliberate `v2.0.0` program.

**Superseded** by the F-54 decision (2026-09-23, [Open Work](Open%20Work.md)):
no `/v1` and no compatibility promise before the product's own 1.0 gate.
Breaking changes have shipped since without a major version on purpose,
for example the removal of `member:impersonate` and RFC 9457 problem
bodies on every error (#1062). The `vN.0.0` tags number clusters; they are
not semver.

## Documentation

### Retro is mandatory; release tag never cut without it

**Decision.** Every cluster ends with a `[X.retro]` PR. The tag
gets cut only after the retro PR merges. The retro updates
`docs/Capabilities.md`, `CHANGELOG.md`, `README.md`,
`docs/Architecture.md`, `docs/Roadmap.md`, and
`docs/Retros/README.md` (the index).

**As practiced now.** The retro is still mandatory, but a retro does not
imply a tag: tagging triggers the release pipeline and is the
maintainer's call, so many clusters are merged and untagged (see
`CLAUDE.md` for the ranges). `docs/Architecture.md` is version-neutral
and changes only when the shape does. A retro updates
`docs/Retros/Cluster X.md` and its index, `docs/Capabilities.md`,
`CHANGELOG.md`, and the Open Work and Roadmap pointers.

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

**Superseded.** The docs are published as an mdBook site
(`book/`, `book/sync-docs.sh`), whose link check fails the build on a dead
internal link. Reference and integrator pages use ordinary Markdown links
(`scripts/check-docs-presentation.sh` refuses wikilinks on every top-level
reference page); wikilinks survive in the historical cluster, retro and
handoff records, which the sync rewrites or leaves out.

### OIDC human login deferred to `v2.0.0` (spike in `v1.4.2`)

**Decision.** `v1.4.0` ships bootstrap hardening (`MAIDAN_BOOTSTRAP`) and an
OIDC **design document** ([OIDC](OIDC.md)) only. Runtime OIDC login, session cookies,
and identity tables land in **`v2.0.0`**.

**Alternative.** Ship OIDC in `v1.4.0` alongside bootstrap gating; or defer
both doc and code to `v2.0.0`.

**Why this:** OIDC adds a new trust boundary (browser sessions, IdP claims,
CSRF/PKCE) on top of the stable bearer-token API. A minor release should not
break MCP/WS clients or semver-stable HTTP auth. The spike unblocks planning
and threat-model updates without half-implemented login.

**To revisit:** if a deployment needs browser login before `v2.0.0`, use an
external reverse proxy (OAuth2 Proxy) in front of `/ui/` only — documented in
[OIDC](OIDC.md) as a stopgap, not a supported Maidan API.

**Superseded.** OIDC login shipped at `v2.0.0`: authorization code with
S256 PKCE, ES256 JWKS validation, sessions and provider logout, tested
against a real loopback provider. See [OIDC](OIDC.md).

## Product scope

### Fidelity + context flagship arc — the optional tail is declined (`v331.0.0`)

**Decision.** The fidelity + context flagship arc is **complete**.
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

### Trace context is transport metadata, not event content (#1099)

**Decision.** A W3C `traceparent` is accepted on every HTTP request (REST,
the WebSocket upgrade, MCP, A2A JSON-RPC) and on the A2A gRPC server. The
work runs as a child of that span, or as a new root when the header is
missing or does not parse. The response names the server span in
`traceresponse`. That same span — not a further child — is what outbound
calls send, so the callee's parent is the Maidan span. The span is stored
on `maidan_events` and copied onto webhook, egress and automation rows at
enqueue, because a bus consumer and the outbox relay run on other tasks
and a task-local does not survive them. It is not part of the content hash.

**Alternatives.** Keep the trace only in a task-local. That dies at the
first `tokio::spawn` and at the outbox relay, which is the gap this
closes. Put the trace inside the hashed event payload. That would make
the same facts hash differently depending on who wrote them, and it would
make the trace part of the domain.

**To revisit.** If a caller needs the trace of a span that is not the one
that wrote the event (a later read, a manual replay), the columns are the
write's span only.

### The wall-clock budget is the thread's total worked time (2026-10-01)

**Decision.** `max_wall_secs` counts every second an agent worked a thread,
from acknowledging a claim to the claim's end, however the claim ends: a
release, an unassign, a reassignment, a freeze, a SCIM deactivation, a
budget stop, a close, or a lapsed lease (#1139). `claim_next` does not hand
out a thread over any of its budgets until the budget is raised or reset.

**Alternative.** Charge only lapsed leases, as #1139 did, so the budget
bounds hung agents only.

**Why this:** a budget that a release resets is not a budget. An agent that
releases and reclaims would never reach it, and an operator reading
`used_wall_secs` would see less than was spent.

**Status.** Landed. Every ending charges, and `claim_next` will not hand
out a thread that is over any budget. A freeze still lasts until an unfreeze;
an expiry on a freeze was not part of this decision.

**To revisit:** if an operator needs a per-claim limit as well as a total.

### A member handle is unique regardless of case (2026-10-01)

**Decision.** Within a workspace, `Alice` and `alice` are the same handle.
Creation by REST, SCIM or the CLI refuses a case-only duplicate, lookups and
mentions fold case, and the SCIM `userName eq` filter is a store query. The
migration that adds the index renames all but the oldest member in each
existing case-only group, so an upgraded database still boots.

**Alternative.** Case-sensitive handles, as before. But SCIM's filter already
matched case-insensitively, so two members could both match one `userName`.

**Why this:** identity providers treat `userName` as case-insensitive, and a
person typing `@alice` means the same person as `@Alice`.

**Status.** Being built (Open Work, Now: lane IDN).

### Artifacts are erased, never soft-deleted (2026-10-01)

**Decision.** Removing an artifact means erasing this workspace's reference
(`DELETE /artifacts/:sha`), and the bytes go when the last reference does.
There is no per-workspace tombstone. The `tombstoned_at` column on
`maidan_artifacts`, which no code outside tests ever wrote, is dropped along
with every check of it.

**To revisit:** when a workspace needs to hide an artifact without deleting
it. That is a per-reference flag, not a column on the shared row.

**Status.** Being built (Open Work, Now: lane IDN).

### A SCIM group grants nothing (2026-10-01)

**Decision.** A SCIM group records membership (#1133) and grants no
capability or channel. If an operator asks for IdP-driven access, the first
step is channel membership from a group, before any capability templates.

**Why this:** a grant that follows an IdP group moves authority outside the
audited grant paths. Nobody has asked for it yet.

### An OIDC browser session can move a thread (2026-10-01)

**Decision.** The capability set an OIDC session carries gains
`thread:transition`, so a person can start a review and close a task from
the board. The separation-of-duties checks apply unchanged: an approval may
be borrowed, never self-approved. A token's session (#1142) carries the
token's own capabilities and reaches the bearer routes except MCP.

**Alternative.** Keep moving a thread token-only, so a person signed in
through an identity provider can read and post but not decide.

**Why this:** the board puts Approve and Close in front of a person. A
session that cannot press them is a broken page, not a safer one.

**Status.** The OIDC half is Open Work Next, write paths for a signed-in
person. The token-session half is #1142.

### Read notifications age out; the usage ledger is kept (2026-10-01)

**Decision.** `MAIDAN_RETENTION_NOTIFICATIONS_DAYS`, off by default, prunes
read notifications older than that through the retention sweeper. Unread
and snoozed notifications are never pruned. The usage ledger has no
retention, since it is a billing record.

**Why this:** read notifications are the one table that grows with every
mention and has no reader after it is read.

**Status.** Being built in #1165.

### F-48 Tier 1 is one delegation list, not a shared SQL dialect (#1100)

**Decision.** The Postgres and SQLite trait impls are generated from one
`store_delegations!` list. Adding a method adds it to both backends; leaving
it off either side does not compile. The body is the same call. Postgres
reads use `read_pool` (the replica, when the request may); SQLite's
`read_pool` is its only pool. `write_lsn` calls `current_wal_lsn`, which is
the one inherent method that differs: Postgres returns a WAL position and
SQLite returns none.

**Alternatives.** A `Dialect` trait that rewrites `$n` versus `?` and
picks a `now()` (Tier 2). That is a behavior change waiting on a stable
schema, and it is not required to stop the impls drifting. Merging the
claim algorithms (Tier 3: outbox, mail outbox, messages, egress) was
rejected: they are different concurrency models, not two copies.

**To revisit.** Tier 2, once the migration stream slows down.

### Maidan shapes and measures model spend; it never proxies model calls (2026-10-01)

**Decision.** Maidan reduces what agents spend on models through the bytes it
serves (stable, layered, content-addressed context; small, stable tool
profiles), the timing of the work it releases (warm then fan out, claims
inside the cache TTL, a batch lane), and the ledger it keeps (every cache tier,
priced, per completed task). It does not call models, hold provider keys,
proxy requests, or cache model responses. Nothing is shared across workspaces.

**Alternatives.** An LLM gateway in front of agents' provider calls, or a
semantic response cache.

**Why this:** a gateway is a crowded business outside the room's job, and it
would make Maidan hold every tenant's provider keys. Response caching returns
stale or wrong answers on agentic traffic, by its own vendors' account. The
savings a coordinator can make are the ones only it can see: who reads what,
what is about to start, what is duplicated, and what a finished task cost.
Sharing across tenants would open a timing side channel.

**Status.** Program C in Open Work; the design is [Context
Economics](Context%20Economics.md). Lanes CTX1 and CTX2 are building C1 and
C2.

**To revisit:** if a self-hosted deployment wants Maidan to emit routing
headers on the agents' behalf (C11), which is still advice, not a proxy.

### MCP caching hints and `server/discover` are implemented (2026-10-01)

**Decision.** Maidan returns `ttlMs` and `cacheScope` on every cacheable MCP
result and implements `server/discover`. This reverses the Cluster 303
disposition that called them optional.

**Why this:** the 2026-07-28 schema requires both fields on `DiscoverResult`,
the resource, prompt and tool lists and `ReadResourceResult`
(`schema/2026-07-28/schema.ts`, `CacheableResult`), and makes `server/discover`
a MUST. Maidan advertises that protocol version. The official SDK clients
cache by those hints, so they also decide how often a client re-fetches
Maidan's 93 KB tool list.

**Status.** Lane CTX2.

### A shared schema expands, then contracts (2026-10-02)

**Decision.** When more than one server version runs against one database, a
schema change is two releases. The first only expands: a new table, a
nullable or defaulted column, an index, or a constraint the previous binary
still satisfies. The second, a later release, contracts: it drops, renames,
retypes, or rewrites, and only after the previous binary is gone. A
compatibility shim does not stay past the contract. A migration that cannot
expand is a cutover, and the previous binary stops before it runs. The rule
is [Migrations](Migrations.md). It does not add an HTTP or MCP compatibility
promise (F-54).

**Alternative.** Keep shipping drops and rewrites in the migration that the
new binary applies on boot, and rely on surge (`maxUnavailable: 0`) to hide
the overlap. Surge is the overlap: the new binary migrates while the previous
one is still serving.

**Why this:** the runner applies migrations on boot, one transaction per
version, under a Postgres advisory lock. Readiness keeps traffic off a
replica that is still migrating. It does not keep the previous binary off the
new schema.

**Status.** Written. The runner is unchanged.

### A token budget counts fresh tokens; cache reads count only in dollars (2026-10-03)

**Decision.** A thread's `max_tokens` counts what the model processed fresh:
uncached input, output and cache writes. Cache reads count toward
`max_usd_micros` at their own price and are shown in the budget's tier
breakdown, but do not count toward `max_tokens`.

**Alternative.** Count every tier one-for-one, which is what the removed
`TokenUsage::total()` did.

**Why this:** at Anthropic, OpenAI, Gemini, Bedrock and Mistral a cache read
costs a tenth of an input token or less (0.05x on Opus 5.5, 0.025x on Fable
5.1), and at xAI 0.15x to 0.25x, as of 2026-10-01 (R2). An agent that re-reads
cached context on every turn would otherwise exhaust its token budget four to
forty times faster than its spend, which punishes exactly
the behaviour Program C asks for. The dollar budget already prices reads.

**Status.** Enforced. `TokenUsage::fresh` is what `max_tokens` counts.
Cache reads are stored on the budget's tier breakdown and priced only into
`max_usd_micros`.


### A profile lists tools the token cannot call; the call is refused (2026-10-03)

**Decision.** `POST /mcp/worker` and `POST /mcp/reviewer` serve a fixed
`tools/list`, sorted by name and byte-identical for every caller, with
`cacheScope: "public"`. The list is not filtered by the token. A tool the
token lacks the capability for is refused at `tools/call`, the same check
`POST /mcp` makes. A tool that is not in the profile is refused on that
endpoint. The full catalog on `POST /mcp` stays filtered and `private`.

**Alternative.** Filter each profile by the token, as `/mcp` does. Then two
workers with different tokens cannot share a cached tool prefix.

**Why this:** the profile exists so a fleet can cache one tool list. A list
that changes with the token cannot be `public`, and two tokens share a prefix
only up to the first tool one of them lacks. Authorization stays at call time,
where it already is for a client that cached a list. A token sees names it
cannot use. That is the cost of a shared list, and a hint never authorizes
the call.

**To revisit:** if a profile grows large enough that showing unusable tools
costs more than the shared cache saves.

### The change flow runs on the maintainer's PAT, behind hard guards in code (2026-10-03)

**Decision.** A Slack `!change` becomes a draft PR.
- Pi codes without any GitHub credential.
- Maidan commits the diff to the named branch and opens the draft PR.
- The credential is David's personal PAT, as `MAIDAN_GITHUB_TOKEN`, with `contents:write` and `pull_requests:write` on the allowed repos.

The token is an admin's, so the guards live in Maidan's code, not in GitHub settings:
- The branch must match `^feature/agent-[a-z0-9][a-z0-9-]*$`.
- The branch is never `prod`, `main`, `master`, `staging` or `dev`.
- The base must be the repo's allowed base.
- `prod` is refused for every repo, whatever the configuration.
- Maidan never merges, marks ready, changes settings, deletes a branch or force-pushes.
- The token is never logged.

Agent PRs are authored as David, so nothing may depend on his approval. The
flow stops at a draft PR, and Soundcheck marks it ready. The shared Maidan
runs in Pi's dev-tools compose stack, built from `main`, not on a hosted
platform.

**Alternatives.** A GitHub App installation token (kept as optional Open Work
Next 25); credentials in Pi's sandbox (refused: Pi holds no write credential
by design).

**Status.** Open Work Next 1 (the delivery) and Next 2 (the from-`main`
instance).

### The connected-apps program runs its fast track without an authorization server (2026-10-03)

**Decision.** The maintainer answered the connected-apps strategy's four questions on 2026-10-03.
- Hosting is hybrid, open source first. Self-hosting is the product. A hosted demo instance serves directory reviewers, with uptime and seeding obligations, and listing copy says plainly that production is self-hosted.
- The OAuth authorization server is on hold. The program is lanes 1 to 7. Lane 8 is parked, not cancelled, and is revisited after the fast track ships and measures. The Claude full-OAuth and ChatGPT listings wait on that revisit. The Muse API-key and Claude static-headers paths are unaffected.
- No enterprise track until the consumer lanes measure. Lane 12 stays parked, and a real customer in one ecosystem reopens its track early.
- Muse shadows Dawn. Lane 6 waits on the outcome of the maintainer's Dawn submission, with its materials prepared. A Dawn rejection on the shared API-key auth model re-scopes Lane 6 to OAuth with PKCE or parks it. A rejection on Dawn-specific grounds changes nothing.

On 2026-10-04 the maintainer added three rulings.
- Nothing is published or submitted to any directory, catalog or registry until the maintainer says go, and every submission is a version with a validation record first. Lane 9's hint and title pass moves ahead of every submission.
- The hybrid hosting decision does not limit the hosted console. The console work in progress continues.
- Maintainer hours are not a constraint, so the program carries no capacity number and no support-hour pause triggers.

**Why.** The lanes that need no authorization server reach about twenty listings for 12 to 22 engineering days. The authorization server and its two gated listings cost 35 to 63 days for one new directory and one auth upgrade, plus permanent operations for a solo maintainer. Self-hosting keeps operations near zero, and the demo instance answers the reviewers.

**Record.** Open Work, the connected-apps rows of Next and Later CA.

### The UI audit's three product questions (2026-10-04)

**Decision.** The maintainer ruled on the three questions the UI deep dive of 2026-10-03 raised.
- The review gate stays opt-in. A thread closes with no approval unless a gate is configured, and the board shows such a close as closed without review.
- `start_review` needs a posted result on gated threads only. On other threads it is allowed, and the approval card warns that no result was posted.
- The web UI ships as a generated, minified bundle checked into the repo. CI fails when the bundle is stale, as with the vendored gRPC code, so `cargo build` still needs no Node.

**Why.** Opt-in keeps the room usable for work that needs no review, while the board makes an unreviewed close visible. A gated thread exists to judge a result, so a review with nothing to judge is refused there. A checked-in bundle gives fingerprinted, minified assets without making Node a build dependency.

**Record.** Open Work Next 3 and Next 9, Later U, and Recently decided.

