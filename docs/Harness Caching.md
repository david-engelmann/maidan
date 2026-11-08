# Harness caching

How to keep Maidan's bytes in a model provider's prompt cache, and how to make
a gateway's spend line up with Maidan's threads. Maidan makes no model calls
and cannot set a breakpoint, a cache key or a session header itself; the
harness or your own client has to. The four SDKs give the same helpers for
that, and this page says where each harness puts Maidan's bytes.

Read [Context Economics](Context%20Economics.md) for why: a hosted cache reuses
a byte-identical prefix, scoped to one provider account and model, and one
changed byte invalidates everything after it. The provider facts below were
checked against each provider's or harness's own documentation on 2026-10-04.
Where only the harness's source code says something, the line says so and
names the commit. Where nothing confirms it, it says UNVERIFIED.

## The boot prefix

Every agent of a channel shares the channel's boot pack: the workspace id, the
glossary, the channel id and the accepted decisions. `GET /channels/{id}/boot`
returns it, and a thread's context pack starts with the same bytes. It is the
part of Maidan's context that a team can share in a provider's cache.

`channels.boot(channelId)` (TypeScript, Python, Rust; `Channels.Boot` in Go)
returns the bytes as served, as text, and their sha256. Use the text as it
came: parsing it and serializing it again can change the bytes. Put the sha256
in `evidence.pack_sha256` when you report the call's usage, so the ledger can
tell which pack a call used.

`cachedPrefix(provider, text, { ttl })` (`cached_prefix` in Python and Rust,
`CachedPrefix` in Go) returns the request part that puts the prefix first and
marks it cacheable:

| Provider | What the helper returns | Where it goes |
|---|---|---|
| `anthropic` | a text block with `cache_control: {"type": "ephemeral"}`, plus `"ttl": "1h"` for the 1-hour tier | first block of `system` |
| `bedrock-converse` | `[{"text": …}, {"cachePoint": {"type": "default"}}]`, plus `"ttl": "1h"` | start of `system` |
| `openai-responses` | a `developer` message whose `input_text` carries `prompt_cache_breakpoint: {"mode": "explicit"}` | first item of `input` |
| `gemini` | `{"parts": [{"text": …}]}` | `systemInstruction` |
| `openai-chat`, `deepseek`, `mistral`, `xai`, `vllm` | `{"role": "system", "content": …}` | first message |

- **Anthropic** caches the prefix up to the breakpoint, in the order tools,
  system, messages. A write costs 1.25x the input price on the 5-minute tier and
  2x on the 1-hour tier; a read costs 0.1x on most models (Anthropic, "Prompt
  caching"). A prefix shorter than the model's minimum, 512 to 4,096 tokens, is
  not cached and no error says so. A boot pack alone is often shorter than that,
  so it is cached only as part of a longer stable prefix.
- **Bedrock** takes a `cachePoint` in Converse. The 1-hour TTL is listed only for
  some models, and Bedrock documents a `ValidationException` when a model that
  supports only 5 minutes gets a `ttl` field. Bedrock's batch inference does not
  cache (AWS, "Prompt caching for faster model inference").
- **OpenAI** takes explicit breakpoints on GPT-5.6 and later, on `input_text`,
  `input_image` and `input_file` blocks; the top-level `instructions` field
  cannot carry one, which is why the helper returns an input message. The TTL is
  30 minutes. Whether an earlier model accepts the `prompt_cache_breakpoint`
  field or rejects it is UNVERIFIED; on those models caching is implicit and the
  prefix only has to come first (OpenAI, "Prompt caching").
- **Gemini, DeepSeek, Mistral, xAI and vLLM** cache a matching prefix without a
  marker. Putting the prefix first is all a request can do.

A harness puts its own system prompt and tools before anything you add, so in a
harness the boot prefix never sits at token 0. It still shares a cache entry
with every agent whose harness bytes before it are identical: the same build,
the same tools and the same settings.

## One cache key per shared-prefix group

Some providers route or partition their cache by a key the request carries.
`cacheKey(workspaceId, group)` (`cache_key`, `CacheKey`) derives one key per
shared-prefix group: `maidan-` and the first 32 hex characters of
`sha256(workspaceId + "\n" + group)`. The workspace id is hashed in, so the
same group in two workspaces gives two keys, and a key reveals neither. A good
group is the channel id, the scope of the boot prefix. Never reuse a key across
workspaces: a shared cache across tenants is a timing side channel.

`cacheKeyFields(provider, key)` says where the key goes:

| Provider id | Field | What the provider says it does |
|---|---|---|
| `openai-responses`, `openai-chat` | body `prompt_cache_key` | Before GPT-5.6 a stable key routes related requests to the same cache; "keys influence routing; they do not pin requests to a machine or guarantee a cache hit". Aim for about 15 requests a minute per key and split busier groups deterministically. On GPT-5.6 and later the key is optional, for "separate cache accounting", and "not needed to optimize caching" (OpenAI, "Prompt caching") |
| `mistral` | body `prompt_cache_key` | "Set the same `prompt_cache_key` on requests that are likely to share a prefix." Mistral's cache TTL and scope are not documented (Mistral, "Prompt caching") |
| `xai-chat` | header `x-grok-conv-id` | Routes requests with the same id to the same server, where the cache lives (xAI, "Maximizing cache hits") |
| `xai-responses` | body `prompt_cache_key` | "Functions identically to setting `x-grok-conv-id`" (same page) |
| `deepseek` | body `user_id` | Isolates KV cache, scheduling and content-safety review per value. Agents meant to share a cache must send the same `user_id`. Allowed characters are `[a-zA-Z0-9-_]`, up to 512 (DeepSeek, chat completion reference) |
| `deepseek-anthropic` | body `metadata.user_id` | The same, on DeepSeek's Anthropic-format API |
| `vllm` | body `cache_salt` | Injected into the hash of the first block, "ensuring that only requests with the same salt can reuse cached KV blocks" (vLLM, "Automatic prefix caching") |
| `anthropic`, `bedrock-converse`, `gemini` | none | No key; the prefix and the account scope decide |

## The thread id as the gateway session id

A gateway sees every model call and its cost, but not which task the call
served. Passing the Maidan thread id as the gateway's session id lets the
gateway's spend be joined to the thread's outcome, which only Maidan knows.
`gatewaySession(gateway, threadId)` (`gateway_session`, `GatewaySession`):

| Gateway | Field | Notes |
|---|---|---|
| `openrouter` | body `session_id` (or the `x-session-id` header) | At most 256 characters. It is also OpenRouter's sticky-routing key, so calls of one thread stick to one upstream provider; two threads may land on different providers and not share a cache (OpenRouter, "Prompt caching") |
| `helicone` | headers `Helicone-Session-Id`, `Helicone-Session-Path` (default `/`), `Helicone-Session-Name` (default `maidan`) | Helicone documents all three as required (Helicone, "Sessions") |
| `litellm` | body `litellm_session_id` | Groups the proxy's logs by session (LiteLLM, "Session logs"). Spend by tag (`metadata.tags`) is an enterprise feature and is not set |
| `tensorzero` | body `tensorzero::episode_id` | The OpenAI-compatible endpoint's name for the episode |
| `tensorzero-native` | body `episode_id` | TensorZero's own inference API. Either form must be a UUIDv7 ("If you must supply your own, generate a UUIDv7", TensorZero, "Episodes"). Maidan thread ids are UUIDv7; the helper refuses any other id |

## Message threads

Anthropic's Messages API has a beta, message threads (beta header
`message-threads-2026-08-12`), in which a request sends only the new turns and
the API holds the rest of the conversation in memory for a short time
(https://platform.claude.com/docs/en/build-with-claude/message-threads).
Maidan makes no model calls, so it never creates or continues one; the harness
or your own client owns the thread. This section says how a threaded harness
should treat Maidan's bytes while it works a task. Every fact about threads
below was checked against that page on 2026-11-08; where a line is our
reading and not the page's, it says so.

Two things are called a thread here. A Maidan thread is the task and its
durable record. A provider thread is one agent's conversation, held by the API
for minutes and dropped without notice. Neither stands in for the other.

### Your transcript, Maidan's pack

- **The harness keeps its own transcript.** The page says the application
  "stays the source of truth for the conversation": keep your own copy of
  every message and build the replay path from the start. No response says
  whether the API stored the turn; the next continue request finds out.
- **`thread_not_found` means replay.** A continue the API cannot serve returns
  404 with `error.details.error_code` set to `thread_not_found`, and does not
  say why. The page's causes: the thread expired; the API served the previous
  turn but did not store it (a response that ends in a tool call cut off by
  `max_tokens`, for one); a fallback model wrote the previous response; the
  previous response is still generating; the request comes from another
  organization or workspace, or from another user or service account than the
  one that created the thread; or the new messages add something the stored
  conversation can't take, such as the first image. Recover by sending the
  full conversation from your copy in a new create request, with `system` and
  `tools`. A model that does not exist is also a 404, so read the `error_code`
  before you replay.
- **The room part of a replay comes from Maidan.** The context pack from
  `get_thread_context` or `GET /threads/{id}/context` is the durable source for
  what the room said, and the boot from `channels.boot` for the channel's
  shared prefix. A provider thread is never the record of a task: it holds no
  more than what one agent sent, and it is gone within the hour. To catch up,
  ask for a delta with `since_prefix_sha` and `message_cursor`. A cursor and a
  sha that do not rebuild the same prefix get the full replacement prefix,
  never a slice that would not hash (`frame_delta` in
  `crates/maidan-types/src/pack.rs`).

### Create and continue

- **System and tools go on the create request only.** "Sending either one on a
  continue request, even as `null`, returns a 400 error." The boot prefix from
  `cachedPrefix` (`cached_prefix`, `CachedPrefix`) belongs in the create
  request's `system` and nowhere else; a continue omits it. To change `system`
  or `tools`, for a new boot sha after a decision was accepted or a new tool,
  start a new thread: a create request with the full conversation.
- **Room updates are not this agent's turns.** What other agents post in the
  Maidan thread reaches a model only if the harness puts it in a message. A
  harness that does owns the order: a thread is append-only, each response
  `id` can be continued once, and the API does not check new messages against
  the ones it holds, so a message sent again can appear twice. Append a room
  update in the next user turn after the response it follows, never into an
  earlier turn. A change to the pack's stable layers (a new prefix sha) is a
  create, not an append.

### Fingerprinted fields and breakpoints

The API keeps "a fingerprint of the fixed fields" with the thread and checks
every later request against it. The fixed fields are `model`, `tool_choice`,
`thinking`, `mcp_servers` (apart from each server's authorization token),
`context_management`, `output_config`, the `skills` in `container`, the
top-level `cache_control`, and the set of `anthropic-beta` values. A different
value returns 400 with the code `thread_fingerprint_mismatch`, and the message
names the fields. Sending `fallbacks` on some requests of a thread and not on
others is the same error.

"A request with `thread` can carry at most 4 `cache_control` breakpoints." The
top-level `cache_control` counts as one, and breakpoints on the create
request's `system` and `tools` count again on every continue request;
breakpoints on earlier stored messages do not. One too many returns 400 with
`thread_unsupported_request`. Maidan's helpers already spend breakpoints:
`cachedPrefix` for `anthropic` marks the boot prefix with one, and a harness
usually marks its own tools and system. Count them before the create, since
what the create puts on `system` and `tools` is charged against every later
turn. When you replay, remove the breakpoints on earlier messages from your
copy and keep one on the last block, as the page says.

### Time-outs

A thread "expires 5 minutes after its last request", each stored response
restarts the 5 minutes, and the API deletes it no more than 1 hour after its
last use. A Maidan task waits on people and machines: a review, an approval
gate, a CI run. Those almost always outlast 5 minutes. So resuming a task is
normally a replay, a new create request built from your transcript and a fresh
pack. A live continue is the fast path inside one burst of work, not the plan.

### Where threads can't be used

- **What the page excludes.** Message threads are not ZDR eligible: the API
  holds a thread in memory, encrypted under a server-managed key, and deletes
  it no more than 1 hour after its last use. Organizations with HIPAA
  readiness or with Inference hooks turned on can't use threads. Neither can
  Message Batches (the item ends with an `invalid_request_error`), token
  counting (a 400; count the full conversation instead, with `system` and
  `tools`), compaction (a 400 with the `compaction` parameter or a
  `compaction` block; compaction on demand is not available), or context
  editing other than `clear_thinking_20251015` with `keep: "all"`. The page
  lists the Claude API, Claude Platform on AWS, Claude Models on Google Cloud
  (Anthropic managed) and Microsoft Foundry as where threads run.
- **Gateways.** The page doesn't cover them; this is Maidan's reading. A
  proxy that drops the `thread` field can still answer 200.
  A continue carries only the new turns, without `system` or `tools`, so the
  model then answers with no memory of the conversation, and nothing in the
  response says so. Before threading through a gateway, check that it forwards
  the field and the beta header, and test a continue whose answer depends on
  an earlier turn.
- **Provider SDK helpers.** The SDK thread helper replays the full
  conversation once by itself on `thread_not_found`, `thread_already_continued`
  or `thread_continue_in_flight`, and keeps its copy in memory only while the
  helper object exists. "The SDK's beta tool runner also uses a thread by
  default, which you can turn off when you create the runner." A harness built
  on either may already thread and replay underneath you. Its copy dies with
  the process, so keep your own transcript when the task must outlive it.

### Cost

"Threads do not change pricing or rate limits", and "a thread does not cache
your prompt for you. Prompt caching works as it does without a thread: you
choose where the cache breakpoints go." A thread makes each request smaller
and the time to first token steadier; it does not make tokens cheaper. The
boot prefix, the breakpoints and the cache keys above stay the cost lever.

### One identity per thread

The page ties a thread to the organization, the workspace and the user or
service account that created it, and a thread created with a legacy workspace
key stays with that key. Whatever holds that identity and a response `id` can
continue the thread, and each `id` can be continued once. In a Maidan swarm
where two agents share one API key, either can continue the other's thread, by
a bug or a leaked id, and the owner's next continue then fails with
`thread_already_continued` and has to replay. Where that matters, give each
agent its own key or service account, and keep provider response ids out of
Maidan messages, which every member of the channel can read.

## Recipes

Each recipe says where Maidan's bytes land in the harness's prompt (L0 the
tools prefix, L1 the system prompt, L2 the conversation; see
[Context Economics](Context%20Economics.md), "Where Maidan's bytes land") and how
to keep them shared. The harness facts come from the research of 2026-10-01
(Context Economics research R3, which names the versions and commits), checked
again against the official docs where there are any.

### Claude Code

- **Tools (L0).** MCP tools are deferred behind tool search by default: only
  names and server instructions load at session start. Claude Code keeps the
  tool list of a conversation's first request for the whole conversation, so a
  server connecting later supplies deferred definitions and disturbs nothing
  cached (Claude Code, "Prompt caching"; "MCP").
- **Instructions.** Claude Code reads server instructions. That it renders them
  as a meta user message, sorted by server, up to 2,048 characters, comes from
  reading the 2.1.267 binary and is not in the docs (UNVERIFIED). Keep
  Maidan's instructions stable; an edit changes what every later request sees.
- **Results (L2).** Tool results are messages, cached for that agent's later
  turns once appended. Output is limited to 25,000 tokens by default
  (`MAX_MCP_OUTPUT_TOKENS`), and a text result over 50,000 characters is saved
  to a file (Claude Code, "MCP").
- **Fan out by forking.** A subagent's "first request doesn't read the parent's
  cache, because the two prefixes differ", while a fork "inherits the parent's
  system prompt, tools, and conversation history exactly, so its first request
  reads the parent's cache". In a workflow fan-out of same-prefix agents, Claude
  Code holds all but the first for up to 5 seconds so the rest read the cached
  prefix (Claude Code, "Prompt caching").
- **Usage.** The `claude_code.token.usage` OpenTelemetry metric has `type`
  `input` (excluding cache reads and writes), `output`, `cacheRead` and
  `cacheCreation` (Claude Code, "Monitoring"). Maidan's `POST
  /threads/{id}/usage/otel` takes JSON attributes, not OTLP, so an
  OpenTelemetry collector would have to transform the metric first; no such
  recipe ships yet.

### Claude Agent SDK

The Agent SDK runs the Claude Code binary, so everything above applies.

- **Share one system prompt across a fleet.** Use the `claude_code` preset with
  Maidan's boot text in `append` and `excludeDynamicSections: true`
  (`exclude_dynamic_sections` in Python). The per-user context then moves into
  the first user message, "leaving only the static preset and your `append` text
  in the system prompt so identical configurations share a cache entry across
  users and machines". It needs `@anthropic-ai/claude-agent-sdk` 0.2.98 or
  later, or `claude-agent-sdk` 0.1.58 or later, and is ignored with a custom
  prompt (Agent SDK, "Modifying system prompts").

  ```ts
  const boot = await maidan.channels.boot(channelId);
  const options = {
    systemPrompt: {
      type: "preset",
      preset: "claude_code",
      append: boot.text,
      excludeDynamicSections: true,
    },
  };
  ```

  Agents share the entry only with the same SDK build, the same tools and the
  same channel, so the same boot bytes.
- **Fork over spawn,** for the reason above.

### Codex

- **Tools (L0).** MCP tools go into the Responses `tools` as a namespace, sorted
  inside, and are deferred behind tool search (from the source at
  openai/codex `b44ca87`).
- **Instructions (L0).** Codex "reads the MCP `instructions` field … and uses it
  as server-wide guidance" and asks servers to "keep the first 512 characters
  self-contained" (Codex, "MCP"). In the source they become the namespace
  description, in the tools prefix, so editing Maidan's instructions busts every
  later byte of every Codex session using it.
- **Cache key.** Codex sends the session id as `prompt_cache_key` and no
  breakpoints (source), so two Codex sessions do not share a key. Whether two
  keys share an identical prefix on GPT-5.6 is UNVERIFIED.
- **Results (L2)** are truncated at 10,000 tokens by default (source);
  `tools.<tool>.output_token_limit` sets one tool's budget (Codex, "MCP").
- **Usage.** `codex exec --json` and its telemetry report `cached_input_tokens`
  and `cache_write_input_tokens` (source). Whether its input count includes the
  cached tokens is UNVERIFIED, so no normalizer reads it.

### Goose

From the source at aaif-goose/goose `bab8ff6` (v1.53.0); Goose's docs do not
describe its prompt layout.

- **Tools (L0)** are sorted by name, with the comment "Stable tool ordering is
  important for multi session prompt caching". In the CLI, Code Mode is on by
  default and exposes three meta-tools instead of Maidan's tools.
- **Instructions (L1).** Goose puts each server's full `instructions` in the
  system prompt, sorted by extension name, and fixes the date there to the hour
  so the prompt stays cacheable. Maidan's instructions are therefore part of
  every Goose agent's cached system prompt; keep them stable.
- **Breakpoints:** the system block, the last tool and the last two user
  messages, with a 5-minute TTL by default and an optional hour.
- **Results (L2)** over 200,000 characters spill to a file.

### OpenHands

From the source at OpenHands/software-agent-sdk `fad6377` (v1.50.1).

- **Tools (L0)** are sent with their raw MCP names in the order the server
  lists them. Every request carries every tool; there is no tool search.
- **Instructions** are not used.
- **Breakpoints** sit on the static system block and the last user or tool
  message. The dynamic system block is left unmarked "to enable
  cross-conversation prompt caching".
- **Cache key.** On OpenAI, `prompt_cache_key` is the conversation id, and
  `prompt_cache_retention` defaults to 24 hours.
- **Results (L2)** are cut at 50,000 characters, and the condenser rewrites
  history after the first two events, which ends prefix reuse past that point.

## Sources

- Anthropic, "Prompt caching": https://platform.claude.com/docs/en/build-with-claude/prompt-caching
- Anthropic, "Message threads": https://platform.claude.com/docs/en/build-with-claude/message-threads
- AWS, "Prompt caching for faster model inference": https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-caching.html
- OpenAI, "Prompt caching": https://developers.openai.com/api/docs/guides/prompt-caching
- Mistral, "Prompt caching": https://docs.mistral.ai/studio-api/conversations/advanced/prompt-caching
- xAI, "Maximizing cache hits": https://docs.x.ai/developers/advanced-api-usage/prompt-caching/maximizing-cache-hits
- DeepSeek, chat completion reference: https://api-docs.deepseek.com/api/create-chat-completion
- vLLM, "Automatic prefix caching": https://docs.vllm.ai/en/latest/design/prefix_caching.html
- OpenRouter, "Prompt caching": https://openrouter.ai/docs/guides/best-practices/prompt-caching
- Helicone, "Sessions": https://docs.helicone.ai/features/sessions
- LiteLLM, "Session logs": https://docs.litellm.ai/docs/proxy/ui_logs_sessions
- TensorZero, "Episodes": https://www.tensorzero.com/docs/gateway/guides/episodes
- Claude Code, "Prompt caching": https://code.claude.com/docs/en/prompt-caching ; "MCP": https://code.claude.com/docs/en/mcp ; "Monitoring": https://code.claude.com/docs/en/monitoring-usage
- Agent SDK, "Modifying system prompts": https://code.claude.com/docs/en/agent-sdk/modifying-system-prompts
- Codex, "MCP": https://developers.openai.com/codex/mcp
