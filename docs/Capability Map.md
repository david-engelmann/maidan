# Capability map

Bearer tokens carry a JSON array of capability strings. Routes and MCP tools
check the required capability before handling the request.

Canonical machine-readable maps:

- MCP tools: [`contracts/mcp-capability-map.json`](../contracts/mcp-capability-map.json) (keys ⊆ [`contracts/mcp-tool-names.json`](../contracts/mcp-tool-names.json))
- HTTP full map: [`contracts/http-capability-map.json`](../contracts/http-capability-map.json) (every OpenAPI bearer operation + transport appendix)
- HTTP denial samples: [`contracts/http-capability-routes.json`](../contracts/http-capability-routes.json) (table-driven e2e)
- Event production: [`contracts/event-surface-disposition.json`](../contracts/event-surface-disposition.json) (every `EventKind`: REST-only, MCP-only, both, or internal-only, with executable evidence)

CI enforces map ↔ OpenAPI parity via `http_openapi_capability_map_contract`, table-driven HTTP denial via `http_capability_matrix_e2e`, exhaustive event-surface classification via `event_surface_disposition_contract`, and `scripts/check-agent-contract.sh`.

## HTTP (member bearer)

| Capability | Routes / behavior |
|------------|-------------------|
| `workspace:read` | GET workspaces, channels, threads, messages, artifacts, search, events (member), GET `/members/:id/manager-digest`, GET `/workspaces/:wid/events/verify` (hash-chain integrity), GET `/workspaces/:wid/snapshot` (header + `graph_hash`; `include_graph=true` needs `token:admin`), GET `/workspaces/:wid/events/catch-up`, GET `/workspaces/:id/audit`, GET `/workspaces/:id/context`, GET `/workspaces/:id/tombstones`, GET `/workspaces/:id/kind-census`, GET `/messages/:id/backlinks`, GET `/workspaces/:wid/mention-webhook`, GET `/workspaces/:id/room`, GET `/workspaces/:id/handle`, GET `/capability-sets`, `POST /tokens/attenuate` (holder-side; no `token:admin`), group-DM list/get, automation list/DLQ/get, MCP notifications SSE, `POST /mcp/streamable` |
| `workspace:write` | POST channels, threads, messages (mentions, votes), references; POST `/workspaces/:id/purge`; automation replay; slash/FSM hook CRUD; `PUT /workspaces/:wid/mention-webhook`; `PUT /workspaces/:id/handle` |
| `message:post` | POST thread messages, A2A `SendMessage` |
| `thread:transition` | POST thread FSM transitions; MCP `transition_thread` |
| `artifact:upload` | POST `/artifacts`, multipart artifact routes |
| `search:query` | GET workspace search |
| `event:subscribe` | WebSocket `/ws/subscribe` (token in subscribe frame) |
| `token:admin` | Mint/revoke/list API tokens (`GET/POST .../members/:mid/tokens`, `DELETE /tokens/:id`); issue/list/revoke `/workspaces/:wid/share-tickets`; signed workspace export / verify / import; snapshot `include_graph=true` |

## MCP (`POST /mcp` tools/call)

| Capability | Tools |
|------------|-------|
| `workspace:read` | `list_channels`, `list_threads`, `list_messages`, `list_dm_conversations`, `list_reactions`, `list_pins`, `get_artifact_metadata`, `list_slash_commands`, `list_fsm_hooks`, `get_thread_context`, `get_workspace_context`, `get_manager_digest`, `get_log_snapshot`, `catch_up_events`, `verify_event_chain`, `list_tombstones`, `list_message_backlinks`, `get_kind_census`, `list_capability_sets`, `parse_maidan_uri`, `get_room`, `attenuate_token` |
| `workspace:write` | `record_mention`, `cast_vote`, `add_reaction`, `remove_reaction`, `pin_message`, `unpin_message`, `add_reference`, `register_slash_command`, `register_fsm_hook`, `set_workspace_handle` |
| `message:post` | `open_dm_conversation`, `post_dm_message`, `post_message`, `edit_message` |
| `artifact:upload` | `upload_artifact`, `begin_artifact_multipart`, `upload_artifact_multipart_part`, `complete_artifact_multipart`, `abort_artifact_multipart` |
| `search:query` | `search_messages` |
| `thread:transition` | `transition_thread` |
| `token:admin` | `create_share_ticket`, `list_share_tickets`, `revoke_share_ticket` |

MCP protocol methods (not tools):

| Capability | Methods |
|------------|---------|
| `workspace:read` | `resources/read`, `resources/subscribe`, `prompts/get` |

## Federation (peer bearer)

| Capability | Routes |
|------------|--------|
| `federation:ingest` | POST `/a2a/v1/events` |
| `federation:admin` | Peer CRUD |

## A2A protocol (`POST /a2a/v1/rpc`)

| Capability | JSON-RPC methods |
|------------|------------------|
| `message:post` | `SendMessage`, `GetTask` |

## Tests

| Suite | Coverage |
|-------|----------|
| `capability_matrix_e2e.rs` | HTTP search/artifacts, MCP `post_message`, A2A, WS subscribe |
| `mcp_capability_matrix_e2e.rs` | Every MCP tool: deny without cap + pass capability gate with cap |
| `http_capability_map_contract.rs` | HTTP contract uses known capability strings |
