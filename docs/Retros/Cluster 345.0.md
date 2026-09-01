# Cluster 345.0 retro — MCP `post_message` slash-command parity (audit P2)

> Tag **`v345.0.0`**. Phase XXIV (post-gate hardening). **Cluster 14 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The audit flagged that MCP `post_message` silently ignored registered slash commands while REST
`post_message` ran them — the same `/deploy` body dispatched over REST but was stored literally
over MCP. The user chose **parity**: an MCP post now runs registered slash commands too.

- **`maidan_mcp::SlashDispatcher` trait** (`slash_dispatch.rs`) — dependency inversion. Slash
  dispatch lives in `maidan-server` (it needs the webhook client, secret decryption, and re-entry
  into the MCP server), but the `post_message` handler lives in `maidan-mcp`, which `maidan-server`
  depends on, not the reverse. The trait lets the server implement dispatch and inject it.
- **`McpServer::set_slash_dispatcher`** — a `OnceLock` field set once at startup (the server is
  already `Arc`-shared, so the setter takes `&self`). `main.rs` attaches
  `ServerSlashDispatcher::new(state.clone())` after every other `attach_*`.
- **MCP `post_message` rewrite** — resolves the thread, and when the body parses to a slash command
  that is registered for the workspace *and* a dispatcher is attached, runs the Cluster-211 shape:
  provisional insert → dispatch → finalizing edit + `MessagePosted` of the edited message, merging
  the slash metadata (`{slash_command, slash_response}`) like REST.

## Surprises / decisions

- **The no-slash path was upgraded to the atomic outbox too.** The old MCP post did a bare
  `post_message` + a bus-gated `publish_event`; it now uses `post_message_with_event` +
  `publish_stored` (the event is appended in the same transaction, whether or not a bus is
  attached) — matching REST and closing a residual non-atomic write on the MCP path.
- **The `AppState`↔`McpServer` cycle is deliberate and test-free.** `ServerSlashDispatcher` holds an
  `AppState` (which holds the `Arc<McpServer>`), so attaching it forms a reference cycle. It is
  created **only in `main.rs`**, where `AppState` lives for the whole process, so the cycle never
  leaks at runtime and never exists in tests/embedders (which leave the dispatcher unset → MCP
  posts skip slash dispatch, the pre-345 behaviour). Documented on the setter.
- **Re-entrancy is fine.** A slash command with an `mcp_tool` handler re-enters the MCP server; the
  REST path already does REST→dispatch→MCP, so MCP→dispatch→MCP is the same nested-async depth, not
  a new lock.

## Test evidence

`mcp_post_runs_registered_slash_command_via_dispatcher` (a mock `SlashDispatcher`: a `/echo` post
carries the merged `slash_command`/`slash_response`; a plain post and an *unregistered* `/name`
post carry none). Full `maidan-mcp` lib (62) + REST `slash_commands_e2e` + `event_emission_e2e`
green (the atomic-path rewrite is behaviour-preserving on the wire). fmt + strict clippy +
`--all-targets` + bootstrap-strip clean.

## Forward look

Remaining audit items: **P1.5** (egress wire-path tests + LSN replica CI); P2 code-side — the
notification batch insert (the Cluster-344 follow-up), the projector link-management surface, and
the Store trait split.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
