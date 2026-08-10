# Cluster 180.0 retro — DM-thread access is participant-checked everywhere

> Tag **`v180.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc A (security & correctness), part 2. **Closes a real confidentiality gap.**

## What shipped

- `ensure_thread_access` is now DM-participant-aware: for a `__dm__` thread it
  calls the new `ensure_dm_participant` (resolves the DM / group-DM conversation
  for the thread, checks membership) instead of the channel exemption.
  `ensure_message_access` inherits it. Added `can_access_thread` (bool form).
- Migrated every thread/message-scoped content route — thread get/context/
  transition/assignee, message post/get/edit/list/tombstone/purge/mention, social
  vote/reaction/pin, and the A2A post + task-read (upgrading the 179 gate) — from
  `ensure_channel_access(channel_id)` to thread/message access.
- Switched the search + workspace-context filters (REST + MCP) from a
  channel-keyed `can_access_channel` cache to a **thread-keyed** `can_access_thread`
  cache, closing the DM-content leak into aggregate reads.

## Surprises

- **The gap was wider than "one route".** The reported hole was the generic
  thread read route, but the same `__dm__` exemption meant *writes* (post /
  react / pin into someone's DM) and *aggregate reads* (search + workspace
  context returned DM message bodies) leaked too. Closing it properly meant
  moving the whole content surface from channel-scoped to thread-scoped access —
  ~25 call sites across four route files + the A2A ingress + four filter sites.
- **`MessageChain` carries `thread_id`, and a message's access == its thread's
  access** — so all message-scoped sites collapse to `ensure_thread_access(chain.thread_id)`,
  which is both DM-aware and cheaper than `ensure_message_access` (no extra
  `get_message`). That made the migration a near-uniform swap per file.

## Decisions

- **Thread-level, not channel-level, is the right axis for DM.** One `__dm__`
  channel maps to many conversations, so `ensure_channel_access` fundamentally
  can't gate DMs; the check has to live where a thread id is known.
- **Keep `ensure_channel_access`'s `__dm__` exemption** for genuinely
  channel-only callers (channel get, list-threads) — no legitimate caller passes
  the `__dm__` channel id without a specific thread.

## Capability table extension

| Change | Where |
|--------|-------|
| DM/group-DM participant enforcement on generic thread/message routes + aggregate reads | `maidan-auth/src/access.rs` + route/tool gates |

## Risks identified + still open

- **Net risk-reducing.** Closes a confidentiality leak (DM read/write + search +
  context); participants and public/private channels are unchanged (full dm /
  group_dm / channel_access / a2a / search / context suites green). The
  thread-keyed filter cache is slightly less reuse-efficient than channel-keyed
  (more distinct keys) — a query-count item for Arc D, not correctness.

## Forward look

Arc A continues: EventKind three-parser parity guard (181), audit-log coverage
(182), default-on rate limits + body cap (183), dual-write atomicity (184).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
