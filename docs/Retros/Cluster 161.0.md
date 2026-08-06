# Cluster 161.0 retro — private-channel access control over MCP (RBAC part C)

> Tag **`v161.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Extends the RBAC enforcement flip from REST to the MCP tool surface.

## What shipped

- **A single pre-dispatch gate** in `tools::dispatch` (`enforce_channel_access`):
  maps each point-access content tool to its id argument and calls
  `ensure_channel_access` / `ensure_thread_access` / `ensure_message_access`
  before the handler runs — covering `list_threads`, `list_messages`,
  `post_message`, `get_thread_context`, `summarize_thread`, pins, `edit_message`,
  `record_mention`, votes, and reactions.
- **`resources/read`** gates `maidan://threads/{id}` and
  `maidan://channels/{id}` (workspace/artifact resources stay workspace-scoped).
- `mcp_denies_non_members_in_private_channels`.

## What was deferred / not covered

| Surface | Why |
|---------|-----|
| MCP aggregate reads (`search_messages`, `get_workspace_context`, `list_channels`) | They filter a *result set*, not a single target — needs handler `auth` + per-hit/thread filtering. Next cluster. |
| WS subscribe gate, `reference.rs`, DM-via-generic-route, `channel:admin` API | Same follow-up set as 160. |

## Surprises

- **The gate belonged in `dispatch`, not the handlers.** The plan anticipated
  threading `auth` into every MCP content handler (churny, and the handlers take
  varied argument tuples). But `dispatch` already has `auth` and the args are a
  generic `Value` with consistent id field names — so one central gate that reads
  `args["thread_id"]` etc. covers all point-access tools with zero handler
  changes. Aggregate reads are the only ones that genuinely need handler-level
  work (they filter, not gate).

## Decisions

- **Gate by argument field, centrally.** Reading the id from the generic `args`
  keeps enforcement in one auditable place and makes coverage obvious (add a tool
  name to the match). Trade-off: a tool with an unusual id field must be added
  explicitly — acceptable and greppable.
- **`resources/read` inline, not via the tool gate.** Resources are a separate
  JSON-RPC method with URI-shaped targets, so they parse the URI and call the same
  helpers directly.

## Capability table extension

| Capability | Where |
|------------|-------|
| MCP point-access private-channel enforcement | `tools/mod.rs`, `server.rs` (`resources_read`) |

## Risks identified + still open

- **MCP point-access closed; aggregate reads still leak** (search /
  workspace-context / list-channels) — the immediate next cluster. Shipped during
  the GitHub Actions outage; re-run CI on `main` when it recovers.

## Forward look

Next: filter the MCP aggregate reads (thread `auth` into those three handlers),
then the WS subscribe gate + `reference.rs`, and the `channel:admin` membership
API — after which the RBAC arc is complete and arc 2 (perf + CI/CD) begins.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
