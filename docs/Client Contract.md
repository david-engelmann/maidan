> **Status (2026-09-29):** the four SDKs implement this contract and are published at
> 0.1.0 (TypeScript, Python, Go, Rust; see [`sdk/README.md`](../sdk/README.md)).
> 0.3.0 (0.2.0 was never tagged) adds a retry with an `Idempotency-Key` per write,
> auto-paging, typed responses and an error per problem type (§2); it is in the tree, not
> yet tagged. Where this page and a running server's `GET /openapi.json` disagree, the
> server wins.
# Client Contract — frozen SDK surface (v1)

Pin this before writing Python / TypeScript / Rust 0.1. If this
file and a running server's `GET /openapi.json` disagree, the
server plus `contracts/http-capability-map.json` win. Patch this
file. Do not invent routes.

The SDK speaks **REST + WebSocket**. MCP and A2A are other doors
(see [Clients.md](archive/Clients.md) §1). They are listed here so names
stay aligned and so [Client Testing.md](archive/Client%20Testing.md) can
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

## 1. Methods (all four languages, identical names)

Capability column is the server check from
`contracts/http-capability-map.json`.

**The method map.** [`sdk/methods.json`](../sdk/methods.json) lists every
method below with the name each SDK gives it (`threads.set_result` is
`threads.setResult` in TypeScript, `Threads.SetResult` in Go and
`threads().set_result` in Rust). `scripts/check-sdk-parity.py` (CI `lint`)
fails when an SDK omits a mapped method or this page does not document one.
An entry with an `example` is also called offline, in all four SDKs, and must
send the request the map names and decode its reply: Python and TypeScript in
the same script, Go in `sdk/go/parity_test.go`, Rust in
`sdk/rust/tests/parity.rs`. Add a method to the map first, then to the four
clients.

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
| `channels.boot` | `GET /channels/{cid}/boot` | `workspace:read` + channel access. 0.3.0: returns the bytes as served, as text, with their sha256, not a parsed model |

Create body: `{ "name", "private": false }`.

### Threads

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `threads.list` | `GET /channels/{cid}/threads` | `workspace:read` | One page: `limit` (1–500, default 100), `cursor` = the last thread id of the previous page |
| `threads.list_all` | same, page after page | `workspace:read` | **0.3.0.** Auto-paging: TS async iterator, Python generator, Go callback, Rust iterator; `page_size` per request, at most 500 (the server's cap: a larger ask would come back short and end the walk early) |
| `threads.create` | `POST /channels/{cid}/threads` | `workspace:write` | Body `{ "title" }` |
| `threads.get` | `GET /threads/{id}` | `workspace:read` | |
| `threads.context` | `GET /threads/{id}/context` | `workspace:read` | Paginated; used by `examples/rest_maidan.py` |
| `threads.transition` | `POST /threads/{id}` | `thread:transition` | FSM. Not PATCH. Confirm body against OpenAPI |
| `threads.set_result` | `PUT /threads/{id}/result` | `thread:transition` | Pair with `wait_for_result`. A lease that expired, was taken over or was released is 409 `claim-lost` |
| `threads.get_result` | `GET /threads/{id}/result` | `workspace:read` | |
| `claim_next_thread` | `POST /channels/{cid}/threads/claim-next` | `thread:transition` | **Hero.** Readiness + skill + lease aware |
| `renew_claim` | `POST /threads/{id}/claim/renew` | `thread:transition` | Holder-only heartbeat. A lease past its deadline is 409 `claim-lost` even before anything reaped it; stop the work. `acknowledge_claim` and `release_claim` answer the same way. During the boot grace after a server starts, a lapsed lease whose holder and token still match is accepted |
| `release_own_claims` | `POST /workspaces/{id}/claims/release-own` | `thread:transition` | 0.3.0. A restarted waiter frees every live claim its token's member holds in the workspace, once, before its first claim. Optional body `{ "note" }`; answers `{ "released": [...] }`. In all four SDKs |

### Messages

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `messages.list` | `GET /threads/{tid}/messages` | `workspace:read` | |
| `messages.post` | `POST /threads/{tid}/messages` | `message:post` | Body `{ "body" }`; author is the authenticated member. A closed thread takes messages; an archived one is 409 `thread-ended`, a deleted one 410 `gone` |

### Artifacts

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `artifacts.upload` | `POST /artifacts` | `artifact:upload` | Simple upload. Multipart is encore, not 0.1 |
| `artifacts.get` | `GET /artifacts/{sha}` | `workspace:read` | |
| `artifacts.meta` | `GET /artifacts/{sha}/meta` | `workspace:read` | |

### Provisioning

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `members.create` | `POST /workspaces/{wid}/members` | (bootstrap) | The unauthenticated seed route, only on a server built with `bootstrap` |
| `members.list` | `GET /workspaces/{wid}/members` | `workspace:read` | |
| `tokens.mint` | `POST /workspaces/{wid}/members/{mid}/tokens` | `token:admin` | The secret comes back once |
| `tokens.list` | `GET /workspaces/{wid}/members/{mid}/tokens` | `token:admin` | Metadata only |

### Reviews, approval gates, footprint and delegation

Added after 0.3.0 (SDK-001), in the tree, not yet tagged. These carry the
differentiators: nobody is their own reviewer, a person holds the gate, and
no two agents take overlapping work.

| SDK | HTTP | Capability | Notes |
|-----|------|------------|-------|
| `reviews.packet` | `GET /threads/{id}/review-packet` | `workspace:read` | The evidence a reviewer is shown; an approval names its `evidence_root` |
| `reviews.request` | `POST /threads/{id}/reviewers` | `thread:transition` | Body `{ "member_id" }`, a member of the thread's workspace. 204 |
| `reviews.submit` | `POST /threads/{id}/reviews` | `thread:transition` | Body `{ "decision": "approve" \| "request_changes", "note"?, "evidence_root"?, "head_sha"? }`. `approve` needs `evidence_root`, `request_changes` needs `note`. An owner or assignee may submit; the verdict does not count |
| `reviews.list` | `GET /threads/{id}/reviews` | `workspace:read` | Each reviewer's current review |
| `reviews.status` | `GET /threads/{id}/review-status` | `workspace:read` | `required_count`, `approvals`, `approvals_met`, `send_backs`, `max_rounds` |
| `approval_gates.list` | `GET /workspaces/{wid}/approval-gates` | `workspace:read` | The pending gates the caller can read, each with the `request_state` an answer echoes |
| `approval_gates.answer` | `POST /approval-gates/{id}/answer` | `workspace:write` | Body `{ "request_state", "action": "accept" \| "decline" \| "cancel", "content"? }`. Accepting also needs a person's signed-in session or `approval:grant`; nobody accepts a gate they asked for. A second answer is 409 |
| `wait_for_approval_request` | WS, kind `approval_requested` | `workspace:read` + `event:subscribe` | A wait helper (below), not an HTTP method |
| `threads.set_footprint` | `PUT /threads/{id}/footprint` | `thread:transition` | Body `{ "repo", "mode": "exclusive" \| "shared", "items": [{ "kind", "value" }] }`. Only an open thread nobody holds; ended is 409 `thread-ended`, deleted 410 `gone` |
| `threads.get_footprint` | `GET /threads/{id}/footprint` | `workspace:read` | |
| `delegation_grants.create` | `POST /workspaces/{wid}/delegation-grants` | `token:admin` | Body `{ "subject_id", "delegate_id", "capabilities", "purpose", "expires_at" }` |
| `delegation_grants.revoke` | `DELETE /workspaces/{wid}/delegation-grants/{gid}` | `token:admin` | Tokens already exchanged for the grant stop working |
| `tokens.delegate` | `POST /tokens/delegate` | `workspace:read` | With the delegate's own token. Body `{ "grant_id", "capabilities": [], "expires_at"?, "label"? }`; at most the grant's and the caller's capabilities. A delegated token cannot delegate again |
| `tokens.attenuate` | `POST /tokens/attenuate` | `workspace:read` | A weaker token from the caller's: `{ "capabilities", "expires_at"?, "label"? }` |

A gate is opened (`request_approval`) and read by id (`get_approval_gate`)
over MCP only: REST has no route for either, and no event marks a gate
resolved, so the SDKs do not offer them. Do not fake either with REST polling.

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
`channel_id`, `thread_id`, `types`, `consumer_id`). `order=desc` reads
newest first and pages back with `before_id`; it takes no `after_id` or
`consumer_id`, since those page forward. `follow` pages
that route, then cuts over to `subscribe` at the last seen id. A
409 `must_refetch` is CursorTooOld — never clamp onto the remaining
log. The problem body (and the WS / MCP SSE frame) may include
`snapshot` → `/workspaces/{id}/snapshot`. Fetch that hashed
checkpoint, then page `GET /workspaces/{id}/events/catch-up?after_lsn=`
until `has_more` is false, then `follow` from the page head.
`include_graph=true` is `token:admin` (or a federation peer), not
the SDK default.

Taps (webhook, WS, search) verify backfill, drain history before
live, and compare live-ready to the head they are chasing. `Maidan-Room-LSN`
reports the caller's **workspace** head only; a tap with a narrower shape
(a channel, thread or kind filter) compares against its own shape's head, or it
waits on events it will never be sent.

Wait helpers are **not** extra HTTP methods. They wrap `subscribe`:

| Helper | Wait until `kind` |
|--------|-------------------|
| `wait_for_result` | `thread_result_set` |
| `wait_for_mention` | `mention_recorded` |
| `wait_for_ready` | `thread_ready` |
| `wait_for_approval_request` | `approval_requested` |
| (also listen) | `message_posted` |

Canonical kinds: `contracts/event-kinds.json`. Ignore unknown kinds.
Do not fake these with REST long-poll.

MCP already has live-wait tools with the **same names**
(`wait_for_result`, `wait_for_mention`, `wait_for_ready`,
`wait_for_notification`). Frameworks use those. The SDK must not
call them; it uses WS so a bot does not need an MCP host.

---

## 2. Errors and retries

Map non-2xx to the error for its RFC 9457 problem `type`: a base
error carrying the status, `type`, `title`, `detail` and the problem
body as sent, and one subclass (TS, Python), wrapper type (Go,
matched with `errors.As`) or enum variant (Rust) per type the server
documents (the table in [Integration reference](Integration%20Reference.md#problem-types)).
A type the client does not know, or a body that is not a problem, is
the unknown-problem error, never a crash. Honor `Retry-After` on
429 (Cluster 172), and on 503, which the server answers when it sheds load. Treat 409 as conflict (`errors.Is` in Go later;
Python/TS/Rust should still distinguish it). A 409 with
`must_refetch: true` / `type: cursor-too-old` is
`is_cursor_too_old` — fail loud, never clamp the cursor onto
the remaining log. 403 is missing capability or channel access,
not "retry." 409 `claim-lost`, 409 `thread-ended` and 410 `gone` say the
target moved on (the lease is no longer live, the thread ended, the thread
was deleted): stop, never retry.

Writes (POST/PUT/PATCH/DELETE) from an authenticated, non-bypass caller
on the protected router and the `/ui/api` write routes take an optional `Idempotency-Key`
header: 1–255 visible ASCII characters, scoped to the caller. The first
request with a key runs; a retry with the same key and the same method,
path, query and body gets the stored response back with
`Idempotent-Replayed: true` and does not run again. The same key on a
different request is 422 `problems/idempotency-key-reused`; a retry while
the first request is still running is 409
`problems/idempotency-key-in-flight` (retry shortly). A 5xx or a
"not now" 4xx (408, 409, 425, 429) or an SSE response is not kept, so a retry after one
runs again. SCIM and A2A routes ignore the header. Keys last 24 hours.

**SDK 0.3.0 retries and keys.** Every write sends a fresh
`Idempotency-Key` (a UUID) and reuses it on each retry of that call, so
a retry after a lost response gets the first answer instead of writing
twice. The client retries up to `max_retries` times (default 2; 0 turns
retries off) after a failure in transit, 408, 429, 500, 502, 503, 504,
or a 409 `problems/idempotency-key-in-flight`. It waits `Retry-After`
when sent (capped at 60s), else 0.5s·2^n capped at 8s with jitter. Any
other 4xx, including a plain 409, is raised at once. Reads are retried
the same way, without a key. The event backfill pages too:
`workspaces.eventsAll` (TS) / `list_events_all` (Python, Rust) /
`Workspaces.ListEventsAll` (Go), by `after_id`, at most 500 per page.

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
| `cached_prefix` / `cache_key` / `cache_key_fields` / `gateway_session` | 0.3.0. Pure functions, no request: the boot prefix with a cache breakpoint, one cache key per shared-prefix group (never shared across workspaces), and the thread id as a gateway session id. Every SDK agrees on `sdk/cache-fixtures/cases.json` ([Harness Caching](Harness%20Caching.md)) |
| `max_retries` | 0.3.0. Retry budget (default 2): TS `{ maxRetries }`, Python `max_retries=`, Go `Client.MaxRetries`, Rust `.with_max_retries(n)` |
| `new_idempotency_key()` / `retry_delay(...)` | 0.3.0. Exported so callers can reuse the policy |
| `normalize_usage(provider, response)` / `usd_micros(tokens, price)` | 0.3.0. Pure functions, no request: a provider response becomes the `model`, `tokens` and `evidence` of a `report_usage` body, and the charge the server checks. Every SDK and the server's ledger agree on `sdk/usage-fixtures/` (Integration, "Normalizing provider usage") |
| Typed IDs | Thread id is not a channel id at the type level |
| Typed responses | Each method in §1 returns the model for its OpenAPI response schema (`Thread`, `ClaimedThread`, `Message`, `ThreadContext`, `StoredEvent`, …). WS event frames stay JSON objects: their shape follows `kind` |
| Unknown fields | Ignore on REST JSON and WS envelopes. A member a model does not declare must not fail the response (Python and Rust keep it in `extra`); a string enum accepts values it does not list |

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
| `channels.create` | `create_channel` | `workspace:write` |
| `threads.create` | `create_thread` | `workspace:write` |
| `messages.post` | `post_message` | `message:post` |
| `messages.list` | `list_messages` | `workspace:read` |
| `threads.context` | `get_thread_context` | `workspace:read` |
| `renew_claim` | `renew_claim` | `thread:transition` |
| `release_own_claims` | `release_own_claims` | `thread:transition` |
| `wait_for_result` | `wait_for_result` | `workspace:read` |
| `wait_for_mention` | `wait_for_mention` | `workspace:read` |
| `wait_for_ready` | `wait_for_ready` | `workspace:read` |
| `wait_for_notification` | `wait_for_notification` | `workspace:read` |
| `reviews.packet` | `get_review_packet` | `workspace:read` |
| `reviews.request` | `add_reviewer` | `thread:transition` |
| `reviews.submit` | `submit_review` | `thread:transition` |
| `reviews.list` | `list_reviews` | `workspace:read` |
| `reviews.status` | `get_review_status` | `workspace:read` |
| `threads.set_footprint` | `set_thread_footprint` | `thread:transition` |
| `delegation_grants.create` | `create_delegation_grant` | `token:admin` |
| `delegation_grants.revoke` | `revoke_delegation_grant` | `token:admin` |
| `tokens.delegate` | `delegate_token` | `workspace:read` |
| `tokens.attenuate` | `attenuate_token` | `workspace:read` |

Over MCP a target that moved on is a JSON-RPC error with its own code:
`-32012` for a lost claim, `-32013` for an ended thread and `-32014` for a
deleted one, each with `data.code` (`claim_lost`, `thread_ended`, `gone`),
`data.status` and `data.type` naming the REST twin. A lost claim's message
starts with `claim_lost`, as before.

There is **no** MCP create workspace or member. An agent creates a
channel with `create_channel` and a thread with `create_thread` (both
`workspace:write`). Workspace and member bootstrap stay on REST, the CLI,
or the SDK, then MCP for claim / wait / post.

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
