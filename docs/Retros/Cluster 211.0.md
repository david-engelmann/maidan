# Cluster 211.0 retro — the entangled post, and an honest recount of what's left

> Tag **`v211.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 10.

## What shipped

- The regular message-post path migrated to the transactional outbox. No-slash
  posts use `post_message_with_event` (fully atomic); slash posts use a new
  `edit_message_with_posted_event` that commits the finalizing edit **and** the
  `MessagePosted` event (of the edited message) in one tx.

## Surprises / decisions

- **Two paths, because the external call can't be in a transaction.** The slash
  post inserts, then dispatches (possibly an HTTP call to an `http` handler), then
  edits. Wrapping insert→dispatch→edit in one tx would hold a DB transaction open
  across a network round-trip — a non-starter. So the route branches: the common
  no-slash case is one atomic insert+event; the slash case is provisional insert →
  dispatch → atomic edit+event. The residual window (a crash *during dispatch*
  leaves an un-announced message) is inherent to the external call, and it's still
  strictly better than the old always-separate append.
- **Deciding the path needs a pre-insert probe.** `slash_will_run` (does a
  registered command match?) used to be checked *after* the insert; to pick the
  atomic path it moved *before* it. That's the same query, just reordered — and
  only runs when the body actually parses as `/command`.
- **An `edit` that emits `MessagePosted`.** `edit_message_with_posted_event` is a
  deliberately odd shape: an edit that announces a *post*. It models "finalize the
  just-inserted message and announce it in final form" — the event is `MessagePosted`
  because, to a subscriber, this is the message's first appearance.
- **The hand-off oversold the finish line.** The running note said "migrate DM
  posts, then the entangled post, then delete `publish()`." But a grep found
  `publish()` still has callers I hadn't accounted for: message **edit**/**tombstone**,
  **A2A ingest**, and the member / workspace / reference / artifact events, plus the
  federation **relay**. So `publish()` stays; the migration has a real tail. Better
  to recount honestly than to delete a still-used function.

## Capability table extension

| Change | Where |
|--------|-------|
| Regular message-post transactional outbox (`edit_message_with_posted_event` + `message_edits::append_in_tx`; route branch) | `store/*/{messages,message_edits}.rs`, `routes/message.rs` |

## Risks identified + still open

- **Mixed atomicity, real tail remaining** (tracked) — `publish()` still serves
  message edit/tombstone, A2A ingest, and member/workspace/reference/artifact
  events. The federation relay re-publishes remote events (not a local domain
  write) and likely stays on a direct publish.

## Forward look

The message *post* paths (regular + DM + A2A-ingest-next) converge on the outbox;
next are message **edit**/**tombstone**, the **A2A ingest** post, and the
peripheral mutations (member/workspace/reference/artifact). Once those land,
`publish()`'s only caller is the federation relay — at which point it's renamed to
its true role rather than deleted. Program A then finishes with federation ingest
trust policy + an RLS spike, before Programs B/C/D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the
[[Retros/Cluster 210.0]] transactional-outbox refactor.
