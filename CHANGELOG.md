# Changelog

All notable changes to Maidan are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
