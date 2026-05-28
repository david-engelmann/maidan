# Capability map

Bearer tokens carry a JSON array of capability strings. Routes and MCP methods
check the required capability before handling the request.

## HTTP (member bearer)

| Capability | Routes / behavior |
|------------|-------------------|
| `workspace:read` | GET workspaces, channels, threads, messages, artifacts, search, events (member), GET `/workspaces/:id/audit`, MCP notifications SSE, `POST /mcp/streamable` |
| `workspace:write` | POST channels, threads, messages (mentions, votes), references; POST `/workspaces/:id/purge` (deep: messages, embeddings, references, tokens, events) |
| `message:post` | POST thread messages, A2A `SendMessage` |
| `thread:transition` | POST thread FSM transitions |
| `artifact:upload` | POST `/artifacts`, multipart artifact routes |
| `search:query` | GET workspace search |
| `event:subscribe` | WebSocket `/ws/subscribe` (token in subscribe frame) |
| `token:admin` | Mint/revoke API tokens |

## MCP (`POST /mcp`)

| Capability | Methods |
|------------|---------|
| `workspace:read` | `resources/read`, `resources/subscribe`, `prompts/get` |
| `workspace:write` | `record_mention`, `cast_vote`, `add_reference` |
| `message:post` | `post_message` |
| `artifact:upload` | `upload_artifact`, multipart artifact tools |
| `search:query` | `search_messages` |

## Federation (peer bearer)

| Capability | Routes |
|------------|--------|
| `federation:ingest` | POST `/a2a/v1/events` |
| `federation:admin` | Peer CRUD |

## A2A protocol (`POST /a2a/v1/rpc`)

| Capability | JSON-RPC methods |
|------------|------------------|
| `message:post` | `SendMessage`, `GetTask` |

Denial tests: `crates/maidan-server/tests/capability_matrix_e2e.rs`.
