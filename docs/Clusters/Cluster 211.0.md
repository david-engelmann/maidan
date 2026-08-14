# Cluster 211.0 — transactional outbox: the regular message post

**Theme:** Program A (security & correctness round 2), part 10 — migrate the
regular (slash-command-entangled) message-post path to the transactional-outbox
pattern.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v211.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `edit_message_with_posted_event(id, editor, edit, dm_id)` — edit + `MessagePosted` (of the edited message) in one tx | `store/{postgres,sqlite}/messages.rs`, `store.rs`, `*/mod.rs` |
| `message_edits::append_in_tx` — edit-history row on a caller tx | `store/{postgres,sqlite}/message_edits.rs` |
| `post_message` route branches: atomic no-slash vs slash-finalize | `routes/message.rs` |

## Why

`MessagePosted` from the regular post route was the hardest hold-out: the route
inserts a message, then — if the body is a registered slash command — runs
(possibly external) slash-command dispatch that **edits** the message, and the
event must reflect the *post-edit* message. You can't wrap the whole thing in one
transaction (the dispatch may make an HTTP call), so the migration splits it.

## The change

The route resolves `slash_will_run` (is there a matching registered command?) and
the DM linkage **before** inserting, then branches:

- **No slash** (the common case): `post_message_with_event(new, dm_id)` (Cluster
  210) — insert + event in one tx. Fully atomic.
- **Slash**: a provisional `post_message` insert, then the external
  `dispatch_slash_command`, then a new `edit_message_with_posted_event` that
  commits the metadata edit **and** the `MessagePosted` event (carrying the edited
  message) in one tx.

`edit_message_with_posted_event` mirrors `edit` (including the edit-history row on
a body change, via the new `message_edits::append_in_tx`) plus the in-tx event
append.

## Scope note — `publish()` stays

This closes the **message-post** hold-out, but the transactional-outbox migration
is larger than the earlier hand-off implied: `publish()` still has callers —
message **edit**/**tombstone** (`MessageEdited`/`MessageTombstoned`), the **A2A
ingest** post, and the member / workspace / reference / artifact events, plus the
federation **relay** (which re-publishes remote events and isn't a local domain
write). So `publish()` is **not** deleted here; those migrate in follow-up
clusters (the relay likely stays).

## Exit criteria

- A regular post (slash or not) and its `MessagePosted` event commit atomically at
  the point the final message is known — **met**.
- `v211.0.0` tagged.

## Verification & limits

- `event_log::message_post_finalize_with_event_appends_atomically` (store): a
  metadata-only finalize emits a durable `MessagePosted` carrying the edited
  message and no edit-history row; a body-changing finalize records one history
  row; both events durable.
- Behaviour-preserving: `slash_commands_e2e`, `message_content_e2e`,
  `event_emission_e2e`, `thread_context_e2e` + the store suite (both backends) green.
- **Residual window (inherent):** on the slash path the provisional insert commits
  before the external dispatch, so a crash *during dispatch* leaves an inserted
  message with no event — unavoidable without a transactional external call, and
  strictly better than the old fully-separate append. The common no-slash path is
  fully atomic.

## References

- [[Retros/Cluster 211.0]]; `store/*/{messages,message_edits}.rs`,
  `routes/message.rs`. Program: [[Roadmap]] + memory `maidan-next-arc-program`
  (Program A). Continues [[Retros/Cluster 210.0]].
