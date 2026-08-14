# Cluster 213.0 retro — the reuse cluster, and creation events need no resolver

> Tag **`v213.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 12.

## What shipped

- A2A ingest post migrated by **reusing** `post_message_with_event` (it's the
  DM-post shape); member + workspace creation gained
  `create_member_with_event` / `create_workspace_with_event` (`MemberJoined` /
  `WorkspaceCreated`).

## Surprises / decisions

- **A2A ingest was already a solved shape.** It inserts a message and publishes
  `MessagePosted { dm_conversation_id: None }` with no post-insert edit — exactly
  the group-DM post from Cluster 210. So the migration is a *reuse*, not a new
  store method: `post_message_with_event(new, None)`. A good reminder that the
  earlier "batch by resolver" investment pays forward — the third `MessagePosted`
  producer cost almost nothing.
- **Creation events don't resolve scope — they *are* the scope.** Unlike the
  message/thread mutations (which resolve `(ws, channel, thread)` from an id),
  `WorkspaceCreated` carries the new workspace and `MemberJoined` the new member +
  its `workspace_id` — both already in hand from the insert. So the `*_with_event`
  methods are the simplest in the whole migration: insert, build event from the
  returned row, append, commit. No resolver call.
- **The cfg-gate is a real pre-flight, not a footgun this time.** `create_member`
  and `create_workspace` are both `#[cfg(feature = "bootstrap")]`, so their
  `publish` / `Utc` imports were bootstrap-gated too. Swapping to `publish_stored`
  meant updating the gated import and re-checking the `--no-default-features` build
  locally (per the module-split-ripple lesson) — caught nothing new, but the check
  is cheap insurance against a red `bootstrap-strip` job.

## Capability table extension

| Change | Where |
|--------|-------|
| A2A ingest + member/workspace creation transactional outbox (`post_message_with_event` reuse; `create_member_with_event`, `create_workspace_with_event`) | `a2a_agent.rs`, `store/*/{members,workspaces}.rs` |

## Risks identified + still open

- **Mixed atomicity, last targets** (tracked) — `publish()` now serves only the
  **reference** (`ReferenceAdded`) and **artifact** (`ArtifactUpserted`) events,
  plus the federation **relay**.

## Forward look

Cluster 214 takes references + artifacts — the last domain mutations. After that,
`publish()`'s only caller is the federation relay (a re-publish of remote events,
not a local write): rename it to that role rather than delete. Then Program A
finishes with federation ingest trust policy + an RLS spike, before Programs B/C/D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the
[[Retros/Cluster 212.0]] transactional-outbox refactor.
