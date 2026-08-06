# Cluster 164.0 retro — channel:admin membership API (RBAC part F)

> Tag **`v164.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Makes private channels operational; only `reference.rs` remains in the RBAC arc.

## What shipped

- **`channel:admin`** capability (KNOWN, not `default_minted`).
- **REST**: `POST` / `GET /channels/:cid/members`, `DELETE …/:mid` — add (role
  upsert) / list / remove, each `channel:admin`-gated, OpenAPI-documented.
- **MCP**: `add_channel_member` / `list_channel_members` / `remove_channel_member`.
- Wired into both capability maps + matrices; end-to-end e2e.

## What was deferred / not covered

| Surface | Why |
|---------|-----|
| `reference.rs` authorization | The last RBAC surface; needs entity→channel resolution — next cluster. |
| Per-channel admin scoping | `channel:admin` is workspace-wide; scoping it to a specific channel is a future refinement. |

## Surprises

- **The contract machinery is a bijection, not a subset.** Adding the three
  routes to `http-capability-map.json` alone failed
  `openapi_bearer_operations_match_http_capability_map` — the map and the
  OpenAPI bearer-op set must match *exactly*, so the routes also needed
  `#[utoipa::path]` docs + registered schemas. Two matrix tests
  (`http_deny_caps`, MCP `deny_caps`) are exhaustive `match`es that panic on an
  unknown capability, so a new cap needs an arm in each. Good guardrails — they
  force new surfaces to be fully documented and tested.

## Decisions

- **`channel:admin` is workspace-wide, not minted by default.** A management
  capability granted on purpose; the holder administers any channel in the
  workspace. Per-channel scoping would need a channel-scoped token model — out
  of scope.
- **Add is an upsert** (role change is idempotent), matching the store method.

## Capability table extension

| Capability | Where |
|------------|-------|
| `channel:admin` + membership API (REST + MCP) | `capability.rs`, `routes/channel.rs`, `tools/channel.rs` |

## Risks identified + still open

- **RBAC is now operational and enforced across read/write (REST+MCP), events
  (WS+MCP SSE), and management** — only `reference.rs` (metadata links) remains.
  Shipped during the GitHub Actions outage; re-run CI on `main` when recovered.

## Forward look

Cluster 165 guards `reference.rs` (add the missing `ensure_workspace` + channel
access), completing the RBAC arc — after which arc 2 (perf + CI/CD) begins.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
