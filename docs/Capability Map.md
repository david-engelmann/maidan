# Capability map

Bearer tokens carry a JSON array of capability strings. Routes and MCP tools
check the required capability before handling the request.

Canonical machine-readable maps:

- MCP tools: [`contracts/mcp-capability-map.json`](../contracts/mcp-capability-map.json) (keys ⊆ [`contracts/mcp-tool-names.json`](../contracts/mcp-tool-names.json))
- HTTP full map: [`contracts/http-capability-map.json`](../contracts/http-capability-map.json) (every OpenAPI bearer operation + transport appendix)
- HTTP denial samples: [`contracts/http-capability-routes.json`](../contracts/http-capability-routes.json) (table-driven e2e)

CI enforces map ↔ OpenAPI parity via `http_openapi_capability_map_contract`, table-driven HTTP denial via `http_capability_matrix_e2e`, and `scripts/check-agent-contract.sh`.

## HTTP (member bearer)

| Capability | Routes / behavior |
|------------|-------------------|
| `workspace:read` | GET workspaces, channels, threads, messages, artifacts, search, events (member), GET `/workspaces/:id/audit`, GET `/workspaces/:id/context`, GET `/workspaces/:wid/mention-webhook`, group-DM list/get, automation list/DLQ/get, MCP notifications SSE, `POST /mcp/streamable` |
| `workspace:write` | POST channels, threads, messages (mentions, votes), references; POST `/workspaces/:id/purge`; automation replay; slash/FSM hook CRUD; `PUT /workspaces/:wid/mention-webhook` |
| `message:post` | POST thread messages, A2A `SendMessage` |
| `thread:transition` | POST thread FSM transitions |
| `artifact:upload` | POST `/artifacts`, multipart artifact routes |
| `search:query` | GET workspace search |
| `event:subscribe` | WebSocket `/ws/subscribe` (token in subscribe frame) |
| `token:admin` | Mint/revoke/list API tokens (`GET/POST .../members/:mid/tokens`, `DELETE /tokens/:id`) |

## MCP (`POST /mcp` tools/call)

| Capability | Tools |
|------------|-------|
| `workspace:read` | `list_channels`, `list_threads`, `list_messages`, `list_dm_conversations`, `list_reactions`, `list_pins`, `get_artifact_metadata`, `list_slash_commands`, `list_fsm_hooks` |
| `workspace:write` | `record_mention`, `cast_vote`, `add_reaction`, `remove_reaction`, `pin_message`, `unpin_message`, `add_reference`, `register_slash_command`, `register_fsm_hook` |
| `message:post` | `open_dm_conversation`, `post_dm_message`, `post_message`, `edit_message` |
| `artifact:upload` | `upload_artifact`, `begin_artifact_multipart`, `upload_artifact_multipart_part`, `complete_artifact_multipart`, `abort_artifact_multipart` |
| `search:query` | `search_messages` |

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
