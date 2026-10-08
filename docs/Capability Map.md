# Capability map

Bearer tokens carry a JSON array of capability strings. Routes and MCP tools
check the required capability before handling the request.

Canonical machine-readable maps:

- MCP tools: [`contracts/mcp-capability-map.json`](../contracts/mcp-capability-map.json) (keys ⊆ [`contracts/mcp-tool-names.json`](../contracts/mcp-tool-names.json))
- HTTP full map: [`contracts/http-capability-map.json`](../contracts/http-capability-map.json) (every OpenAPI bearer operation + transport appendix)
- HTTP denial samples: [`contracts/http-capability-routes.json`](../contracts/http-capability-routes.json) (table-driven e2e)
- Event production: [`contracts/event-surface-disposition.json`](../contracts/event-surface-disposition.json) (every `EventKind`: REST-only, MCP-only, both, or internal-only, with executable evidence)

CI enforces map ↔ OpenAPI parity via `http_openapi_capability_map_contract`, table-driven HTTP denial via `http_capability_matrix_e2e`, exhaustive event-surface classification via `event_surface_disposition_contract`, and `scripts/check-agent-contract.sh`.

> **This page itself is covered by none of them.** They guard the JSON contracts
> above; nothing compares *this* table to `capability::all()`. That is why five
> live capabilities — `channel:admin`, `secret:read`, `secret:admin`,
> `audit:read-global`, `operator:global` — were missing here until 2026-09-23
> while being correct in the contracts the entire time. A guard is scheduled in
> Cluster 411.1; until it lands, add new capabilities here by hand.

**Delegation lends work, never authority.** A delegation grant may carry only
`workspace:read`, `workspace:write`, `message:post`, `thread:transition`,
`artifact:upload`, `search:query` and `event:subscribe`. Every other capability
is refused at grant creation and stripped from any borrowed token regardless of
how its grant was created. `capability::tests::every_capability_is_classified_as_work_or_authority`
fails if a new capability is added without deciding which it is.

## HTTP (member bearer)

| Capability | Routes / behavior |
|------------|-------------------|
| `workspace:read` | GET workspaces, channels, threads, messages, artifacts, search, events (member), GET `/members/:id/manager-digest`, GET `/workspaces/:wid/events/verify` (hash-chain integrity), GET `/workspaces/:wid/snapshot` (header + `graph_hash`; `include_graph=true` needs `token:admin`), GET `/workspaces/:id/audit`, GET `/workspaces/:id/context`, GET `/workspaces/:id/tombstones`, GET `/workspaces/:id/kind-census`, GET `/messages/:id/backlinks`, GET `/workspaces/:wid/mention-webhook`, GET `/workspaces/:id/room`, GET `/workspaces/:id/handle`, GET `/capability-sets`, `POST /tokens/attenuate` and `POST /tokens/{id}/rotate` of the caller's own token (holder-side; no `token:admin`), `POST /auth/session/from-token` (the caller's own token becomes a browser session with its authority), group-DM list/get, automation list/DLQ/get, MCP notifications SSE, `POST /mcp/streamable` |
| `workspace:write` | POST channels, threads, messages (mentions, votes), references; automation replay; slash/FSM hook CRUD; `PUT /workspaces/:wid/mention-webhook`; `PUT /workspaces/:id/handle` |
| `message:post` | POST thread messages, A2A `SendMessage`; `PATCH /messages/:id` and `DELETE /messages/:id` on **your own** message — only the author can edit a message (no capability lets anyone rewrite another member's words), and tombstoning someone else's needs `channel:admin` |
| `thread:transition` | POST thread FSM transitions; claim the next ready thread in a channel (`POST /channels/:cid/threads/claim-next`) or anywhere in the workspace the caller may read (`POST /workspaces/:wid/threads/claim-next`); experimental `POST /threads/:id/land-gate/advice` when enabled; MCP `transition_thread` |
| `artifact:upload` | POST `/artifacts`, multipart artifact routes |
| `search:query` | GET workspace search |
| `event:subscribe` | WebSocket `/ws/subscribe` (token in subscribe frame) |
| `channel:admin` | Channel membership (`GET`/`POST /channels/:cid/members`, `DELETE …/members/:mid`); granting and revoking **governance** skills (`land_gate`, `review`) on another member via `/members/:id/skills`; per-thread review controls (`DELETE /threads/:id/land-gate`, `…/review-requirement`, `…/reviewers/:member_id`); and moderation — tombstoning another member's message (`DELETE /messages/:id`) |
| `secret:read` | `GET /workspaces/:wid/secrets`, `POST /workspaces/:wid/secrets/:name/resolve` |
| `secret:admin` | `POST /workspaces/:wid/secrets`, `DELETE /workspaces/:wid/secrets/:name`; the secret-egress allowlist: `GET /workspaces/:wid/secret-egress-hosts`, `DELETE …/secret-egress-hosts/:host`, and `POST /workspaces/:wid/secret-egress-hosts`, which **also needs `secret:read`** (a listed host receives the values, so a token that may rotate secrets but not read them cannot read them by listing a host) |
| `audit:read-global` | `GET /operator/audit` — cross-workspace audit read |
| `operator:global` | `GET /operator/legal-holds`, `GET /operator/status`; an instance-wide `POST /operator/reindex-embeddings` (no `workspace_id`) and reading its job (a workspace-scoped reindex is `workspace:write`) |
| `approval:grant` | Accept an approval gate (`POST /approval-gates/:id/answer` with `accept`) with a token, or a session made from one. Without it, accepting needs a browser session a person signed in to through the identity provider, sent from the console page (`POST /ui/api/approval-gates/:id/answer`); a plain bearer token whose member is a human does not accept. Declining and cancelling need only `workspace:write`. In no preset and never delegatable: an admin grants it deliberately to an automated approver the workspace trusts. Through MCP `approval_decide` it accepts only a gate whose risk is below the workspace's `approval-policy` threshold (default `low`, so none); every other accept the tool asks for returns a one-time link the bound person confirms at `POST /ui/api/approval-confirmations/confirm` with their signed-in session |
| `token:admin` | Mint/revoke/list API tokens (`GET/POST .../members/:mid/tokens`, `DELETE /tokens/:id`); rotate another member's token (`POST /tokens/:id/rotate`); create/list/revoke `/workspaces/:wid/delegation-grants`; `PUT /workspaces/:wid/delegation-policy` (the grant-lifetime ceiling); `PUT /workspaces/:wid/approval-policy` (the risk at which a model's accept needs a person, audited; `GET` is `workspace:read`); `PUT /workspaces/:wid/retention` (the workspace's own retention, audited; `GET` is `workspace:read`); issue/list/revoke `/workspaces/:wid/share-tickets`; signed workspace export / verify / import; snapshot `include_graph=true`; **destroying the record** — `POST /workspaces/:id/purge`, `DELETE /workspaces/:id` (erase), `DELETE /messages/:id/purge`, `DELETE /artifacts/:sha` (erase this workspace's reference); legal holds — `POST/GET /workspaces/:id/legal-holds`, `DELETE /workspaces/:id/legal-holds/:hold_id`, and `GET /workspaces/:id/legal-holds/preserved` (audited); GET `/workspaces/:wid/events/catch-up` (the whole log as one chain, every private channel and DM included; or a registered federation peer); SCIM 2.0 provisioning at `/scim/v2/` — `Users` (including a `userName` rename) and `Groups`, confined to the token's workspace. The SCIM routes answer in SCIM's own error envelope and sit outside OpenAPI and `http-capability-map.json`; each handler checks `token:admin` itself |

## MCP (`POST /mcp` tools/call)

| Capability | Tools |
|------------|-------|
| `workspace:read` | `list_channels`, `list_threads`, `list_messages`, `list_dm_conversations`, `list_reactions`, `list_pins`, `get_artifact_metadata`, `list_slash_commands`, `list_fsm_hooks`, `get_thread_context`, `get_workspace_context`, `get_manager_digest`, `get_log_snapshot`, `verify_event_chain`, `list_tombstones`, `list_message_backlinks`, `get_kind_census`, `list_capability_sets`, `parse_maidan_uri`, `get_room`, `attenuate_token`, `rotate_token`; the long-poll waits `wait_for_mention`, `wait_for_notification`, `wait_for_result`, `wait_for_ready`, `wait_for_claim_expired`, `wait_for_claim_failed`, `wait_for_blocked_resolved`, `wait_for_landed`, `wait_for_memory_block` |
| `workspace:write` | `record_mention`, `cast_vote`, `add_reaction`, `remove_reaction`, `pin_message`, `unpin_message`, `add_reference`, `register_slash_command`, `register_fsm_hook`, `set_workspace_handle`, `approval_decide` (declines on this alone; accepts directly only with `approval:grant` below the workspace threshold, else returns a confirmation link) |
| `message:post` | `open_dm_conversation`, `post_dm_message`, `post_message`, `edit_message` (author only) |
| `artifact:upload` | `upload_artifact`, `begin_artifact_multipart`, `upload_artifact_multipart_part`, `complete_artifact_multipart`, `abort_artifact_multipart` |
| `search:query` | `search_messages` |
| `thread:transition` | `transition_thread`, `claim_next_thread`, `claim_next_workspace_thread` |
| `channel:admin` | `add_channel_member`, `list_channel_members`, `remove_channel_member`, `clear_land_gate` |
| `secret:read` | `list_secrets`, `resolve_secret` |
| `secret:admin` | `list_secret_egress_hosts`, `revoke_secret_egress_host`, and `allow_secret_egress_host` (also needs `secret:read`) |
| `token:admin` | `create_share_ticket`, `list_share_tickets`, `revoke_share_ticket`, `create_delegation_grant`, `list_delegation_grants`, `revoke_delegation_grant`, `set_delegation_policy`, `set_retention_policy`, `freeze_member`, `unfreeze_member`, `list_frozen_members`, `export_workspace`, `verify_workspace_export`, `import_workspace`, `catch_up_events` (the whole log as one chain, every private channel and DM included) |

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

The HTTP+JSON binding (`/a2a/v1/...`) checks the same capabilities.

| Capability | JSON-RPC methods |
|------------|------------------|
| `message:post` | `SendMessage`, `SendStreamingMessage`, `GetTask`, `ListTasks`, `CancelTask`, `SubscribeToTask`, `GetExtendedAgentCard` |
| `workspace:write` | the four push-config methods; a `SendMessage` that opens a new context thread |

## Tests

| Suite | Coverage |
|-------|----------|
| `capability_matrix_e2e.rs` | HTTP search/artifacts, MCP `post_message`, A2A, WS subscribe |
| `mcp_capability_matrix_e2e.rs` | Every MCP tool: deny without cap + pass capability gate with cap |
| `http_capability_map_contract.rs` | HTTP contract uses known capability strings |
