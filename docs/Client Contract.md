> **Status (2026-09-29):** the four SDKs implement this contract and are published at
> 0.1.0 (TypeScript, Python, Go, Rust; see [`sdk/README.md`](../sdk/README.md)).
> 0.2 adds a retry with an `Idempotency-Key` per write and auto-paging. Where this page
> and a running server's `GET /openapi.json` disagree, the server wins.
# Client Contract — frozen SDK surface (v1)

Pin this before writing Python / TypeScript / Rust 0.1. If this
file and a running server's `GET /openapi.json` disagree, the
server plus `contracts/http-capability-map.json` win. Patch this
file. Do not invent routes.

The SDK speaks **REST + WebSocket**. MCP and A2A are other doors
(see [Clients.md](Clients.md) §1). They are listed here so names
stay aligned and so [Client Testing.md](Client%20Testing.md) can
re-run the same operations over those transports.

Auth on every REST call: `Authorization: Bearer {token}`.
WebSocket: bearer in the subscribe frame, not only a query
string. Constructor takes `base_url` and `token`. Never mint
`token:admin`.

Ignore unknown JSON fields and unknown WS `kind` strings
(forward-compat).

---

## 0. Transports for the same operations

| Door | When | How the v1 SDK treats it |
|------|------|--------------------------|
| REST + WS | Default. Agent they wrote. Slack adapter later | **This is the SDK.** Methods below |
| MCP | LangChain / AutoGen / Cursor / any MCP host | Not a method. `client.mcp_url` is `{base_url}/mcp/streamable`. Tool names in §6 must stay twins of the SDK names. The server negotiates `2026-07-28` by default and `2025-11-25`, `2025-06-18`, `2025-03-26` or `2024-11-05` on request |
| A2A | Another vendor's agent | Not in the v1 SDK. Use any A2A v1.0 client; `examples/a2a_interop.py` is a walkthrough. Agent Card at `GET /.well-known/agent-card.json` |
| Webhooks | n8n / Zapier | Not in the v1 SDK. REST `POST /workspaces/{wid}/webhooks` already exists. OpenAPI is the contract |

Do not add a fourth protocol. Do not wrap MCP or A2A as the
primary SDK transport. A2A clients use an A2A SDK; the server passes
the official A2A TCK (`scripts/a2a-tck.sh`) apart from the cases listed,
with reasons, in `scripts/a2a-tck/exclusions.txt`.

---

## 1. Methods (all three languages, identical names)

Capability column is the server check from
`contracts/http-capability-map.json`.

### Workspaces

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `workspaces.create` | `POST /workspaces` | (bootstrap / write) | Body `{ "name" }`. Confirm against OpenAPI; not every gate lists this path. Hero seed is REST/CLI; there is **no** MCP create-workspace tool |
| `workspaces.get` | `GET /workspaces/{id}` | `workspace:read` | |
| `workspaces.import` | `POST /workspaces/import` | `token:admin` | **Admin-only.** Expose it, but do not hide that the caller token must be admin. Not part of the agent hero loop |

There is **no** `workspaces.list`. Do not invent one.

### Channels

| SDK | HTTP | Capability |
|-----|------|------------|
| `channels.list` | `GET /workspaces/{wid}/channels` | `workspace:read` |
| `channels.create` | `POST /workspaces/{wid}/channels` | `workspace:write` |

Create body: `{ "name", "private": false }`.

### Threads

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `threads.list` | `GET /channels/{cid}/threads` | `workspace:read` | One page: `limit` (1–500, default 100), `cursor` = the last thread id of the previous page |
| `threads.list_all` | same, page after page | `workspace:read` | **0.2.** Auto-paging: TS async iterator, Python generator, Go callback, Rust iterator; `page_size` per request |
| `threads.create` | `POST /channels/{cid}/threads` | `workspace:write` | Body `{ "title" }` |
| `threads.get` | `GET /threads/{id}` | `workspace:read` | |
| `threads.context` | `GET /threads/{id}/context` | `workspace:read` | Paginated; used by `examples/rest_maidan.py` |
| `threads.transition` | `POST /threads/{id}` | `thread:transition` | FSM. Not PATCH. Confirm body against OpenAPI |
| `threads.set_result` | `PUT /threads/{id}/result` | `thread:transition` | Pair with `wait_for_result` |
| `threads.get_result` | `GET /threads/{id}/result` | `workspace:read` | |
| `claim_next_thread` | `POST /channels/{cid}/threads/claim-next` | `thread:transition` | **Hero.** Readiness + skill + lease aware |
| `renew_claim` | `POST /threads/{id}/claim/renew` | `thread:transition` | Holder-only heartbeat |

### Messages

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `messages.list` | `GET /threads/{tid}/messages` | `workspace:read` | |
| `messages.post` | `POST /threads/{tid}/messages` | `message:post` | Body `{ "body" }`; author is the authenticated member |

### Artifacts

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `artifacts.upload` | `POST /artifacts` | `artifact:upload` | Simple upload. Multipart is encore, not 0.1 |
| `artifacts.get` | `GET /artifacts/{sha}` | `workspace:read` | |
| `artifacts.meta` | `GET /artifacts/{sha}/meta` | `workspace:read` | |

### Subscribe (WebSocket)

| SDK | HTTP | Capability |
|-----|------|------------|
| `subscribe` | `GET /ws/subscribe` | `event:subscribe` |
| `list_events` | `GET /workspaces/{id}/events` | `workspace:read` |
| `snapshot` (not in 0.1) | `GET /workspaces/{id}/snapshot` | `workspace:read` (`include_graph=true` needs `token:admin`) |
| `catch_up` (not in 0.1) | `GET /workspaces/{id}/events/catch-up` | `token:admin` or a federation peer (the whole log; an agent token reads `GET /workspaces/{id}/events`, filtered to what it may see) |
| `follow` | HTTP backfill then WS cutover | `workspace:read` + `event:subscribe` |

Subscribe frame: `contracts/ws-subscribe-filter.schema.json`
(`workspace_id` enables replay; optional `channel_id`, `thread_id`,
`member_id`, `kinds[]`, `channel_grants[]`). `after_id` and
`consumer_id` are siblings of `filter` on the subscribe frame, not
inside it. Server replies `subscribe_ack`, `schema_version`,
`resume_token`, `after_id`, `room_lsn` (the filter's workspace head, or `null`
when the subscription spans workspaces). `type: cursor_too_old` is **not** a
benign control frame — deliver it and stop.

Live frames and REST `GET /workspaces/{id}/events` (`StoredEvent`)
carry `$type` (`maidan.event.{kind}/1`) in addition to `kind`. Each
row also carries `{lsn, prev_hash, content_hash}` (Cluster 392 hash
chain; `lsn` equals `id`). Ignore unknown fields. The pack is
`contracts/lexicon/`. `$type` is not a stored column.

REST responses and webhook POSTs stamp `Maidan-Room-LSN`: the caller's (or the
subscribing) workspace's event-log high-water, in decimal.
Clients expose `last_room_lsn` / `LastRoomLSN` / `lastRoomLsn` from
that header and **must not** parse a `Maidan-Consistency-Token` WAL
string (`0/hex`) as a room head. The two headers answer different
questions (projector lag vs read-your-writes).

`list_events` accepts projector-shape query params (`after_id`,
`channel_id`, `thread_id`, `types`, `consumer_id`). `follow` pages
that route, then cuts over to `subscribe` at the last seen id. A
409 `must_refetch` is CursorTooOld — never clamp onto the remaining
log. The problem body (and the WS / MCP SSE frame) may include
`snapshot` → `/workspaces/{id}/snapshot`. Fetch that hashed
checkpoint, then page `GET /workspaces/{id}/events/catch-up?after_lsn=`
until `has_more` is false, then `follow` from the page head.
`include_graph=true` is `token:admin` (or a federation peer), not
the SDK default.

Taps (webhook, WS, search) verify backfill, drain history before
live, and compare live-ready to the **workspace / shape** head, which is what
`Maidan-Room-LSN` reports.

Wait helpers are **not** extra HTTP methods. They wrap `subscribe`:

| Helper | Wait until `kind` |
|--------|-------------------|
| `wait_for_result` | `thread_result_set` |
| `wait_for_mention` | `mention_recorded` |
| `wait_for_ready` | `thread_ready` |
| (also listen) | `message_posted` |

Canonical kinds: `contracts/event-kinds.json`. Ignore unknown kinds.
Do not fake these with REST long-poll.

MCP already has live-wait tools with the **same names**
(`wait_for_result`, `wait_for_mention`, `wait_for_ready`,
`wait_for_notification`). Frameworks use those. The SDK must not
call them; it uses WS so a bot does not need an MCP host.

---

## 2. Errors and retries

Map non-2xx to a single error type that includes HTTP status and
the JSON body the server already returns. Honor `Retry-After` on
429 (Cluster 172), and on 503, which the server answers when it sheds load. Treat 409 as conflict (`errors.Is` in Go later;
Python/TS/Rust should still distinguish it). A 409 with
`must_refetch: true` / `type: cursor-too-old` is
`is_cursor_too_old` — fail loud, never clamp the cursor onto
the remaining log. 403 is missing capability or channel access,
not "retry."

Writes (POST/PUT/PATCH/DELETE) from an authenticated, non-bypass caller
on the protected router take an optional `Idempotency-Key`
header: 1–255 visible ASCII characters, scoped to the caller. The first
request with a key runs; a retry with the same key and the same method,
path, query and body gets the stored response back with
`Idempotent-Replayed: true` and does not run again. The same key on a
different request is 422 `problems/idempotency-key-reused`; a retry while
the first request is still running is 409
`problems/idempotency-key-in-flight` (retry shortly). A 5xx or a
"not now" 4xx (408, 409, 425, 429) or an SSE response is not kept, so a retry after one
runs again. SCIM and A2A routes ignore the header. Keys last 24 hours.

**SDK 0.2 retries and keys.** Every write sends a fresh
`Idempotency-Key` (a UUID) and reuses it on each retry of that call, so
a retry after a lost response gets the first answer instead of writing
twice. The client retries up to `max_retries` times (default 2; 0 turns
retries off) after a failure in transit, 408, 429, 500, 502, 503, 504,
or a 409 `problems/idempotency-key-in-flight`. It waits `Retry-After`
when sent (capped at 60s), else 0.5s·2^n capped at 8s with jitter. Any
other 4xx, including a plain 409, is raised at once. Reads are retried
the same way, without a key. The event backfill pages too:
`workspaces.eventsAll` (TS) / `list_events_all` (Python, Rust) /
`Workspaces.ListEventsAll` (Go), by `after_id`.

---

## 3. Hero-loop capabilities

An agent token for the README snippet needs:

- `message:post`
- `workspace:read`
- `event:subscribe`
- `thread:transition`

Not `token:admin`. `artifact:upload` only if the cookbook uploads.
`workspace:write` only if the cookbook creates channels/threads
(the 278 loop usually seeds those via CLI / bootstrap).

---

## 4. Constructor extras (not methods)

| Extra | Spec |
|-------|------|
| `MAIDAN_URL` / `MAIDAN_TOKEN` | Default constructor inputs; explicit args win |
| `client.mcp_url` | `{base_url}/mcp/streamable`. String only. No MCP dependency |
| `last_room_lsn` | Last seen `Maidan-Room-LSN` (decimal). Not a WAL token |
| `event_type(kind)` | `maidan.event.{kind}/1` |
| `max_retries` | 0.2. Retry budget (default 2): TS `{ maxRetries }`, Python `max_retries=`, Go `Client.MaxRetries`, Rust `.with_max_retries(n)` |
| `new_idempotency_key()` / `retry_delay(...)` | 0.2. Exported so callers can reuse the policy |
| Typed IDs | Thread id is not a channel id at the type level |
| Unknown fields | Ignore on REST JSON and WS envelopes |

---

## 5. Out of scope for v1

Search, webhooks, A2A as a library, MCP as a library, OIDC,
apps/oauth, DMs, votes, slash commands, federation, scheduler,
`workspaces.list`, generating the rest of OpenAPI, create-* MCP
tools, `Crew.kickoff`.

Those stay Integration.md / Protocols.md / Framework Integrations.md.

Adding a method is a contract bump (v2), not a silent 0.1.x.

---

## 6. MCP twins (for tests and recipes, not the SDK)

Same operations, MCP tool names from
`contracts/mcp-tool-names.json`. Capabilities from
`contracts/mcp-capability-map.json`. If these drift, CI on the
server already fails; patch this table to match.

| SDK / helper | MCP tool | MCP capability |
|--------------|----------|----------------|
| `claim_next_thread` | `claim_next_thread` | `thread:transition` |
| `messages.post` | `post_message` | `message:post` |
| `messages.list` | `list_messages` | `workspace:read` |
| `threads.context` | `get_thread_context` | `workspace:read` |
| `renew_claim` | `renew_claim` | `thread:transition` |
| `wait_for_result` | `wait_for_result` | `workspace:read` |
| `wait_for_mention` | `wait_for_mention` | `workspace:read` |
| `wait_for_ready` | `wait_for_ready` | `workspace:read` |
| `wait_for_notification` | `wait_for_notification` | `workspace:read` |

There is **no** MCP create workspace / channel / thread / member.
An MCP-only agent cannot bootstrap. Seed via REST, CLI, or the
SDK, then MCP for claim / wait / post.

MCP endpoint: `POST /mcp/streamable`. Pin `mcp>=1.9,<2` in
examples, not in the SDK. Protocol: `2026-07-28` (default).

---

## 7. A2A (a recipe, not an SDK surface)

A2A v1.0 over JSON-RPC and HTTP+JSON, checked by the official A2A TCK
(`scripts/a2a-tck.sh`; excluded cases in `scripts/a2a-tck/exclusions.txt`).
Do not generate an A2A client in 0.1; A2A SDKs already exist.

| Call | HTTP | Note |
|------|------|------|
| Agent Card | `GET /.well-known/agent-card.json` | Spec v1.0 card |
| JSON-RPC | `POST /a2a/v1/rpc` | Every v1.0 operation. Send `A2A-Version: 1.0` |
| HTTP+JSON | `/a2a/v1/message:send`, `/a2a/v1/tasks`, … | Same operations, AIP-193 errors |
| Parts | | Text or URL. File/data parts are J5 |
| Events | `POST /a2a/v1/events` | Federation ingest, not A2A |

Federation card `GET /.well-known/maidan.json` is Maidan-to-Maidan,
not A2A. Do not confuse them.

---

## 8. How to verify before coding

1. Run compose.quickstart.
2. `GET /openapi.json` and confirm every path in the tables above
   exists. If a path moved, fix this file first.
3. Confirm kinds in `contracts/event-kinds.json`.
4. Confirm MCP twins in `contracts/mcp-tool-names.json`.
5. Then implement. Do not implement from memory of an older cluster.
