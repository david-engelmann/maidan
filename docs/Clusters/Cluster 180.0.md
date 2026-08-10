# Cluster 180.0 — security: DM-thread access is participant-checked everywhere

**Theme:** Arc A (security & correctness), part 2 — close the `__dm__` exemption
so a DM/group-DM thread isn't reachable by a non-participant through generic
thread/message routes or aggregate reads.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v180.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `ensure_thread_access` is DM-participant-aware; new `ensure_dm_participant` + `can_access_thread` | `maidan-auth/src/access.rs` |
| Thread/message content routes gate on thread access (not channel access) | `routes/{thread,message,social}.rs`, `a2a_agent.rs` |
| Aggregate reads (search + workspace-context) filter per-thread (DM-aware) | `routes/search.rs`, `routes/workspace.rs`, `mcp/tools/{search,mod}.rs` |

## Why

DM and group-DM threads all live in one `__dm__` system channel per workspace.
The RBAC arc's `ensure_channel_access` **exempted** `__dm__` (a channel-level
check can't identify *which* conversation), delegating DM enforcement to the
dedicated `/dm` / `/group-dms` routes' participant checks. But the **generic**
content routes (`GET /threads/:id`, `/threads/:id/messages`, `/context`, plus
message/reaction/pin/vote routes and the A2A ingress) gated on
`ensure_channel_access(channel_id)` — so any workspace member who knew a DM
thread id could read (and write) it. Aggregate reads had the same hole:
`can_access_channel(__dm__)` returned true, so **workspace search and the
workspace-context pack leaked DM message content** to non-participants.

## The fix

DM access is inherently per-thread, so the check moved to the thread level:
- `ensure_thread_access(thread_id)` now resolves the thread's channel and, when
  it's `__dm__`, calls the new `ensure_dm_participant` (resolves the DM /
  group-DM conversation for that thread and verifies the caller is a member);
  otherwise it does the normal channel check. `ensure_message_access` inherits
  this (it delegates to `ensure_thread_access`).
- All thread/message-scoped content routes (thread get/context/transition/
  assignee, message post/get/edit/list/tombstone/purge/mention, social
  vote/reaction/pin, and the A2A post + task-read) now gate on
  `ensure_thread_access` / (message →) its thread, not `ensure_channel_access`.
  Channel-scoped routes (channel get, list-threads, list-channels) keep
  `ensure_channel_access`.
- Search + workspace-context filters switched from a channel-keyed
  `can_access_channel` cache to a **thread-keyed** `can_access_thread` cache,
  which is DM-participant-aware (and still correct for public/private channels).

## Exit criteria

- A non-participant can't read/write a DM thread via any generic route or see it
  in search / workspace-context; participants + normal channels unchanged; suites
  green — **met**.
- `v180.0.0` tagged.

## Verification & limits

- `dm_thread_not_readable_via_generic_route_by_non_participant` (channel_access_e2e,
  auth enabled): a non-participant gets `403` on `GET /threads/:id` +
  `/threads/:id/messages` for a DM thread; both participants get `200`. The full
  dm / group_dm / channel_access / a2a / search / thread_context suites stay
  green (participants + public/private channels unaffected).
- Limit: `ensure_channel_access` still exempts `__dm__` for genuinely
  channel-only callers (none legitimately pass the `__dm__` channel id without a
  thread). Per-thread filtering trades the channel-decision cache for a
  thread-decision cache (more distinct keys, still one lookup per thread) — a
  query-count consideration for Arc D, not a correctness one.

## References

- [[Retros/Cluster 180.0]]; `maidan-auth/src/access.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (Arc A).
