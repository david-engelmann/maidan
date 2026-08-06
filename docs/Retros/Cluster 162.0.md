# Cluster 162.0 retro — MCP aggregate-read filtering (RBAC part D)

> Tag **`v162.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Closes the MCP aggregate-read leaks; the channel-content vuln is now closed on
> both primary surfaces (REST 160, MCP point-access 161, MCP aggregate 162).

## What shipped

- **`search_messages`** — takes `store` + `auth`, drops hits in channels the
  caller can't access (per-channel cache).
- **`list_channels`** — takes `auth`, hides private channels the caller isn't a
  member of (public + `__dm__` always listed).
- **`get_workspace_context`** — the dispatch arm filters `v["threads"]` by each
  thread's `channel_id` access (cache), leaving `context.rs` untouched.
- Test coverage extended to assert the filtering for a non-member vs member.

## What was deferred / not covered

| Surface | Why |
|---------|-----|
| WS subscribe gate (`subscribe_grants`) | Private-channel *events* still reach a non-member who asserts grants — a distinct mechanism (Cluster 163). |
| `reference.rs` | No ws/access check at all (163). |
| `channel:admin` membership API | The management surface (164). |

## Surprises

- **Two shapes of "filter".** Point tools (161) gate before running; aggregate
  tools filter after. Where the handler returns raw data (`get_workspace_context`
  returns a `Value`), the dispatch arm can filter without touching the handler;
  where the handler wraps in `content_json` (`search`, `list_channels`), the
  filter has to move *into* the handler with `auth`. Picking the cheaper of the
  two per tool kept the churn to three small edits.

## Decisions

- **Cache the per-channel decision** in every aggregate filter — a search page or
  workspace context spanning one channel does a single `can_access_channel`, not
  one per hit/thread.
- **`get_workspace_context` filtered in the dispatch arm**, not `context.rs`,
  to keep the MCP context builder auth-free and shared.

## Capability table extension

| Capability | Where |
|------------|-------|
| MCP aggregate-read channel filtering | `tools/{search,channel,mod}.rs` |

## Risks identified + still open

- **Channel-content read/write is now closed on REST + MCP.** Remaining RBAC
  gaps are the event stream (`subscribe_grants`), `reference.rs`, and the
  management API — the next two clusters. Shipped during the GitHub Actions
  outage; re-run CI on `main` when it recovers.

## Forward look

Cluster 163 verifies WS subscribe grants against `channel_is_member` and guards
`reference.rs`; Cluster 164 adds the `channel:admin` capability + the
`/channels/:cid/members` REST + MCP management API — completing the RBAC arc
before arc 2 (perf + CI/CD).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
