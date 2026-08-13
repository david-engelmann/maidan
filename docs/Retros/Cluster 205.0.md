# Cluster 205.0 retro — the domain write and its event commit together now

> Tag **`v205.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 4 — transactional-outbox foundation.

## What shipped

- The transactional-outbox pattern: `append_in_tx` + `*_with_event` store methods
  that commit a domain row and its event in one transaction, plus a
  `publish_stored` route helper for the post-commit notify. Applied to channel +
  thread create; the rest migrate in follow-up clusters.

## Surprises / decisions

- **The atomic half already existed — one level down.** `append_event` already
  wrote the event *and* its outbox row in a single tx. The gap was never
  event↔outbox; it was **domain-row ↔ event**. So the fix isn't "add an outbox" —
  it's "let the domain mutation join the tx the append already opens". Extracting
  `append_in_tx` (append without the commit) and calling it inside the mutation's
  tx is the whole mechanism.
- **Event construction moves into the store — and that's the real cost.** For the
  append to share the mutation's tx, the event must be built where the tx lives:
  inside the store method. Most events need context the route used to supply
  (`ThreadCreated` needs `workspace_id`), so the store resolves it *in the tx*
  (`SELECT workspace_id FROM channels WHERE id = ?`). That's the pattern every
  migrated mutation will repeat, and it's why this is multi-cluster: ~20 mutations
  each need a `*_with_event` that resolves its own event's context.
- **`publish` split into durable-append vs. notify.** The old `publish` did both:
  append (durable) then bus-notify. Now the append is inside the store tx, so the
  route only needs the notify — `publish_stored`. It hydrates the event from the
  stored payload (the in-memory bus needs the full event; the pg bus would send a
  pointer). Keeping the two concerns separate makes "a bus failure never undoes a
  commit" obvious in the code, not just the comment.
- **Full refactor, so mixed atomicity is temporary — and named.** The user chose
  the full multi-cluster path over incremental, accepting that mid-migration the
  codebase has both atomic (`*_with_event`) and retry-hardened (`publish`) writes.
  That's fine *because it converges* — each cluster moves more mutations across
  until `publish` has no callers and gets deleted. The retro/CHANGELOG name it
  explicitly so it reads as "in progress", not "half-done and forgotten".

## Decisions

- **Start with channel + thread create** — no entanglement, context is trivially
  available, so the pattern is established on the easy cases before the hard ones
  (the slash-edit message-post path).
- **Behaviour-preserving is the safety bar.** The same events must still reach
  subscribers; `event_emission_e2e` staying green is the proof the notify path is
  intact through `publish_stored`.

## Capability table extension

| Change | Where |
|--------|-------|
| Transactional-outbox foundation (`append_in_tx` + `create_{channel,thread}_with_event` + `publish_stored`) | `store/*` + `routes/*` |

## Risks identified + still open

- **Mixed atomicity during the migration** (accepted, tracked) — non-migrated
  mutations still use `publish()`. Each is atomic-or-retry-hardened, so no
  regression; the migration converges. Open: the message-post path is the hard one
  (build the event in-store *after* the slash-command edit so it reflects the final
  message).

## Forward look

The transactional-outbox migration continues (social, transitions, assignments,
then the entangled message-post path) until `publish()` has no callers. Program A
also has **206** (federation ingest trust policy + an RLS spike). Then Programs B
(agentic orchestration), C (notifications & reach), D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Completes the
Cluster 184 dual-write deferral's foundation.
