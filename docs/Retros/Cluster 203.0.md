# Cluster 203.0 retro — no more tailing a DM you're not in

> Tag **`v203.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program A (security & correctness round 2), part 2.

## What shipped

- The subscribe path (`expand_event_filter`) and the DM/group-DM metadata routes
  now enforce participation: a non-participant can no longer tail a DM's live
  messages (via `dm_conversation_id` or the `__dm__` `thread_id`), and a session
  caller can't read/enumerate others' DM metadata. Bearer orchestrator power is
  preserved for the roster reads; message content requires participation for
  everyone.

## Surprises / decisions

- **Two different "act as" models, and the tension is real.** Cluster 202 (writes)
  says a **bearer** may act as any member — the orchestrator model. Cluster 180
  (DM message reads) says even a bearer must be a *participant*. This cluster hit
  both at once, and a naive "gate everything on participation" broke `group_dm_e2e`
  immediately: that test mints a **bot** token (a non-participant orchestrator) and
  lists a member's group DMs — which is *legitimate*. The failure forced the right
  distinction: **roster/list metadata → session-guarded (bearer act-as-any);
  message content → participant-required (bearer included).** A DM's messages are
  more sensitive than its member list.
- **`ensure_thread_access` was the perfect existing tool for the subscribe gate.**
  It's already DM-participant-aware (Cluster 180 resolves the DM/group-DM conversation
  for a `__dm__` thread), so running it on the resolved `thread_id` closes the
  `dm_conversation_id` path *and* the direct-`thread_id` path *and* group DMs, in
  one call, on both transports — no new DM-membership logic.
- **Order-of-operations bug hiding the fix.** `expand_event_filter` ran *before*
  the caller's identity was resolved in `ws.rs`, so the gate had no `auth` to check
  against. Reordering the identity resolution above the expand (it doesn't depend
  on the expanded filter) was the enabling change.
- **The gate belongs in `expand_event_filter`, not the two call sites.** Both WS
  and MCP-SSE call it, so putting the `ensure_thread_access` inside it (not
  duplicated in `ws.rs` + `mcp_stream.rs`) means one place enforces it for both —
  and it fires for a direct `thread_id` filter too, not only DM-id expansions.

## Decisions

- **Reuse `ensure_acting_member` (Cluster 202) for the list endpoints** — "a
  session may only list its own" is exactly that helper's rule, so the model stays
  consistent across writes and DM enumeration.
- **Bypass stays exempt everywhere** — tests and embedders that disable auth are
  unaffected.

## Capability table extension

| Change | Where |
|--------|-------|
| DM/group-DM participation on subscribe + metadata reads | `dm.rs`, `group_dm.rs`, `ws.rs`, `mcp_stream.rs` |

## Risks identified + still open

- **Roster reads remain bearer-open** (orchestrator model, intentional). A bearer
  can read any conversation's participant list — the same act-as-any tradeoff as
  writes (202). Whether that should be tightened is a policy question, documented.

## Forward look

Program A continues: **204** cross-tenant artifact isolation (artifacts have no
`workspace_id` — a known SHA + `workspace:read` reads another tenant's blob),
**205** transactional outbox (atomic domain-write + event-append), **206**
federation ingest trust policy + an optional RLS spike. Then Programs B/C/D.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Completes the
DM-participation surface [[Retros/Cluster 180.0]] opened.
