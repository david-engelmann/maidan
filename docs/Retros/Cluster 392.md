# Cluster 392 retro — Wave 3 #32: hash-chained log + strong refs

Wave 3 #32 (B16) asked for a **hash-chained event log** plus **strong
refs**. Every stored event carries `{id, lsn, prev_hash, content_hash}`.
`claim_next` and A2A citations pin `{uri, content_hash}`. A federated
peer detects a break without trusting the host.

This is **not** MST/CAR / ATProto PDS, **not** Room-LSN (Cluster 390),
**not** WAL `Maidan-Consistency-Token` (Cluster 263), and **not**
signed workspace export (Cluster 391). Those answer lag, replica
routing, and offline-file authenticity. The chain answers "was this
workspace's event log rewritten?"

Four impl PRs (392.1–392.4) + this retro. Every PR targets `main`.
**Row #32 is closed.** Do not start Wave 3 #33–36 from this close.

## What shipped

- **392.1 (#849) — types.** `$type` `maidan.event-log.chain/1`. SHA-256
  (`sha256:<hex>`). `content_hash` over Cluster 391 canonical JSON of
  the Event payload. `prev_hash` = genesis (`SHA-256(b"maidan.event-log.genesis/1")`)
  or `SHA-256(prev || "\n" || content || "\n" || decimal_id)` of the
  previous **workspace** row. `EventLink`, `StrongRef`, `verify_chain`,
  `verify_peer_link`. Hashed, not signed.
- **392.2 (#850) — store.** Columns on `maidan_events` (pg 0091 /
  sqlite 0090). `append_in_tx` mints hashes under a per-workspace
  lock. `Store::verify_event_chain` walks the retained suffix.
  `StoredEvent` gained `lsn` (= `id`), `prev_hash`, `content_hash`.
- **392.3 (#851) — REST + federation.** `GET /workspaces/:wid/events/verify`
  (`workspace:read`; 200 / 409 `event-log-broken`). Federation ingest
  verifies the **origin** envelope before parse/remap, then stores
  origin hashes on `maidan_federated_ingest` (pg 0092 / sqlite 0091).
- **392.4 (#852) — strong refs.** `claim_next` returns `ClaimedThread`
  with a flatten `pin` to the `ThreadAssignmentChanged` event.
  A2A `citations: [{uri, content_hash}]` persist on
  `metadata.citations` and echo on the agent reply.

## Decisions

- **Hashed, not signed.** A wholly fabricated but internally consistent
  chain still verifies. Authorship of a carried file is Cluster 391.
  The chain's job is rewrite-detection for a peer that already has
  prior hashes.
- **`lsn` is the event-log `id`.** Same integer Room-LSN names. Not
  WAL. `FederationEnvelope` validation requires `event.lsn == event.id`.
- **Per-workspace chain.** Unscoped events (`workspace_id IS NULL`)
  are their own chain. Global id gaps are expected.
- **Verify the retained suffix.** Retention may drop a prefix. The
  oldest remaining row need not chain from genesis. Snapshot catch-up
  of that prefix is #33 — out of scope.
- **Verify origin hashes before serde-parse of payload.** Remap
  rewrites `workspace_id` inside the Event; hashing after parse would
  400 on a well-formed envelope. Local append after remap mints **new**
  ids and hashes. Origin hashes live on `maidan_federated_ingest`.
- **Strong-ref URIs** are `maidan:event/{id}`, `maidan:thread/{uuid}`,
  `maidan:message/{uuid}` — not #35 `maidan://{room}/…`.
- **claim_next pin is the assignment event**, not a hash of the Thread
  snapshot. The claim mutation *is* that event.
- **Fail closed.** Broken chain → 409 `event-log-broken`. Broken
  federated origin hash → the same 409 (not 403 — a rewrite is a
  chain break, not a capability miss). Malformed A2A citation → 400.

## Surprises

- **`create_*` without `_with_event` does not append.** The first
  sqlite store test seeded with `create_workspace` / `create_member`
  / `create_channel` and asserted `checked >= 2`. Those helpers write
  domain rows only. Postgres "passed" because Docker was unavailable
  and the test skipped. Seed through `*_with_event`.
- **Hash-verify must run before `event_from_stored`.** Doing it after
  made the first federation e2e fail with 400 (payload rewrite) instead
  of 403 (hash break).
- **`Thread` is not `Eq`.** Do not derive `PartialEq` on `ClaimedThread`.
- **Draft PR #848** (`$type` on REST `GET /events` StoredEvent) **merged**
  while this cluster was in flight. 392.2 originally added hash fields
  via serde defaults, not a custom `Serialize`. After rebase onto
  `main`, `lsn` / `prev_hash` / `content_hash` ride Cluster 848's
  `stored_event_wire` (`$type` stays) and the OpenAPI companion
  schema. New fields are optional on deserialize (the `$type`
  evolution rule).

## Test evidence

- Types: genesis, content hash, chain, tamper, peer sequential /
  genesis / prev-break, URI helpers, claim pin, well-formed hash.
- Store (sqlite always; postgres when Docker): append → verify OK;
  prev-hash rewrite → `PrevHashMismatch`; empty chain OK.
- REST e2e: verify 200 / 409; capability 401.
- Federation e2e: origin payload rewrite 409; sequential prev-hash
  break 409.
- claim_next REST + MCP: pin uri `maidan:event/{id}`, `sha256:` hash
  matching the assignment event.
- A2A: citations persist + echo; empty omitted on the wire; malformed
  400.

## Forward look

**Cluster 392 is complete. Row #32 is closed.**

Do not start Wave 3 #33–36 from this retro. #33 is snapshot catch-up
of a pruned prefix; #34 tombstone explorer; #35 named capability sets
+ `maidan://` URIs; #36 WASI slash-handler.

Deferred: MCP `verify_event_chain` twin (REST is the verify path);
streaming catch-up of a pruned prefix (#33); signing the chain
(Cluster 391 already signs the export envelope).

Do **not** cut `v392.0.0` from this PR — the maintainer tags, which
triggers `release.yml`.

## Acknowledgements

#849 types → #850 store → #851 REST+federation → #852 strong refs →
this retro.
