# R3: The ecosystem around Maidan's context economics

> Research input to [Context Economics](../../Context%20Economics.md), dated 2026-10-01. Kept as evidence: every claim carries its source and access date. The fetched pages and clones it cites were working copies and are not committed; the URLs and `repo@commit` citations make each claim re-checkable.


*Harness prompt construction, protocol caching hooks, the cost-savings market, credible measurement, and what a coordinator could build.*

**Date:** 2026-10-01. **Status:** research input for the context-economics program. Nothing here is a decision.

**Method.**
- **Primary sources.** Vendor docs, spec text and schemas, and source code at pinned commits. The code comes from shallow clones at the pinned commits cited, plus strings from the shipped Claude Code 2.1.267 binary.
- **Dates.** Web pages were accessed on 2026-09-30 or 2026-10-01 unless a publication date is given. A bare URL means "accessed 2026-10-01".
- **Code citations** use `repo@commit path Lnn`.
- **Labels.** **UNVERIFIED** means I could not confirm it in a primary source. **(arithmetic)** means the number is my own calculation from cited prices, not a source's figure.
- **Maidan facts** come from main @ `94836a1c` (2026-09-30), read from a read-only checkout.

---

## 0. Findings that change the plan

1. **Maidan is out of spec on the very MCP features it declined.**
   - **What the spec says.** In MCP `2026-07-28`, `ttlMs` and `cacheScope` are **required**, and `server/discover` is a **MUST**:
     - `schema/2026-07-28/schema.ts` L1081–1110 (spec repo @`046fa30`, 2026-09-28) declares both fields non-optional on `CacheableResult`.
     - The changelog's minor change #5 reads: "Require `ttlMs` and `cacheScope` fields on results returned by `tools/list`, `prompts/list`, `resources/list`, `resources/read`, and `resources/templates/list`" ([SEP-2549](https://github.com/modelcontextprotocol/modelcontextprotocol/pull/2549); https://modelcontextprotocol.io/specification/2026-07-28/changelog).
     - `server/discover`: "Servers **MUST** implement it" (`docs/specification/2026-07-28/server/discover.mdx`).
   - **What Maidan does.** On main, a grep of `crates/maidan-mcp/src` and `crates/maidan-server/src` for `server/discover`, `ttlMs`, `cacheScope`, `subscriptions/listen` and `resultType` returns **zero hits**. `docs/Open Work.md` L207 files these under J3 as "Optional MCP `2026-07-28` features no client relies on".
   - **Both premises are wrong.** The fields are required, and the official SDK clients rely on them: "Nothing on the client opts in: every `Client` holds a response cache, and the server's hint decides what it may serve" (typescript-sdk@`433eb41` `docs/clients/caching.md`; the Python SDK `docs/client/caching.md` is equivalent).
   - **Instructions are affected too.** `instructions` moved into `DiscoverResult` (schema.ts L678–697). Maidan emits them only from `initialize` (`crates/maidan-mcp/src/server.rs` L536–556), so a client running purely on the 2026 handshake never sees them. How the TS SDK's `versionNegotiation: 'auto'` probe handles Maidan's `-32601` reply to `server/discover` is **UNVERIFIED**; it probably falls back to `initialize`.

2. **Maidan's content never sits at token 0 of a hosted-API prompt.**
   - Every harness puts its own system prompt and tools first. Anthropic caches "`tools`, `system`, and `messages` (in that order)" (https://platform.claude.com/docs/en/build-with-claude/prompt-caching).
   - Two agents share a cache entry only if:
     - (a) their prefixes are byte-identical up to and including Maidan's content;
     - (b) they bill to the same Anthropic **workspace** ("Caches are isolated per workspace" on the Claude API, Claude Platform on AWS and Foundry; Bedrock and Vertex isolate per organization) or the same OpenAI organization;
     - (c) the entry is still warm.
   - Claude Code's own docs say "the cache is effectively scoped to one machine and directory" (https://code.claude.com/docs/en/prompt-caching).
   - **Consequence:** a "shared room prefix" across *heterogeneous* harnesses cannot work on hosted APIs. It can work for *homogeneous fleets* that Maidan configures (same harness build, configuration, capability set and API workspace), and on self-hosted engines with prefix caching.

3. **The only Maidan content in every harness's prefix is its tool catalog, and the catalog is large.**
   - **Size.** `catalog()` (`crates/maidan-mcp/src/tools/catalog.rs`) defines **200 tools**, about **96.6k compact JSON characters**, roughly **27.6k tokens** at 3.5 characters per token. That is an estimate from the Rust source; the real tokenizer count is **UNVERIFIED**.
   - **The seven-tool "hero loop"** recommended in `docs/Framework Integrations.md` is about 7.0k characters (≈2.0k tokens, 7.3% of the catalog).
   - **Which harnesses defer MCP tools by default:**
     - Claude Code and the Agent SDK;
     - Codex CLI;
     - pi (codemode is the default exposure);
     - Cursor (tool descriptions written to files).
   - **Which send the full list on every request:** Goose (unless Code Mode is on), the OpenHands SDK, Cline (both bundles), CrewAI, LangChain without middleware, and the OpenAI Agents SDK with local MCP servers.
   - **Cost (arithmetic, Sonnet 5.5 at $2 input / $2.50 5-minute write / $0.20 read per MTok, https://platform.claude.com/docs/en/about-claude/pricing), for a 50-turn task that resends the full catalog:**
     - about $2.76 uncached;
     - about $0.34 with one cache write and 49 reads;
     - about $0.025 for the seven-tool surface.

4. **Server `instructions` land in a different place in every harness.**
   - **Claude Code:** a conversation-level meta user message, capped at 2,048 characters (docs and binary).
   - **Codex:** the MCP namespace description, which sits *inside the tools prefix*.
   - **Goose:** the system prompt, in full.
   - **pi:** one line of at most 250 characters.
   - **Ignored:** OpenHands, Cline, CrewAI and the OpenAI Agents SDK.
   - **LangChain:** available only manually.
   - Maidan's current instructions are 1,305 characters with no newline, so pi will truncate them (§1).

5. **Plain provider caching already captures most of the per-agent hit rate.**
   - TraceLab found a "global prefix cache hit rate is 95.7%" across about 4,300 real Claude Code and Codex sessions (arXiv 2606.30560, 2026-06-29).
   - Terminal-Bench's top row implies about 95.85%. That is arithmetic, and the field semantics are **UNVERIFIED** (§4).
   - Maidan's honest headroom is therefore elsewhere:
     - **cache writes:** a fleet writes a shared prefix once instead of N times;
     - **trajectory length:** better context means fewer turns per success;
     - **avoided duplicate and abandoned work;**
     - **cheaper lanes:** Batch, Flex and off-peak pricing for work whose deadline has slack;
     - **model and effort routing by task class.**
   - A claim that Maidan raises hit rate will be small and should not be the headline.

6. **No one sells dollars per *completed task*.**
   - Gateways bill per key, user, team or session. Memory layers measure tokens against stuffing the full context. Compressors measure tokens removed.
   - A gateway vendor documents that semantic caching "goes badly wrong on agentic traffic" (LiteLLM).
   - Vendors document that multi-agent work costs *more*: about 15× chat tokens (Anthropic research), about 7× for agent teams in plan mode (Claude Code), and five subagents ≈ 5× tokens (Cursor).
   - So the open position is to *measure and lower cost per finished task, and stop paying twice for the same work* (§3).

7. **Usage never reaches an MCP server today.**
   - Every harness *records* cache-read and cache-write tokens, and none sends them to the MCP server.
   - Maidan's `report_usage` therefore depends on agent self-report unless Maidan ingests harness telemetry: Claude Code OTel, Codex `exec --json` or OTel, Goose and OpenHands spans, the Agents SDK `Usage` object, or Cursor's Admin API.
   - The ledger's single `cache_write` tier cannot represent Anthropic's mixed 5-minute (1.25×) and 1-hour (2×) writes in one report (`crates/maidan-types/src/usage_ledger.rs`; §4.5).

8. **Protocol hooks available today:**
   - deterministic `tools/list` order (SHOULD);
   - `ttlMs` / `cacheScope` and `subscriptions/listen` invalidation;
   - the Skills extension, which serves files with sha256 digests (Final, SEP-2640);
   - `ResourceLink` results;
   - A2A Agent Card `ETag` / `Cache-Control` (§8.6);
   - A2A `contextId` as an affinity key;
   - A2A extensions as a place for usage data.

   **Not available yet:** ETags on MCP primitives (roadmap), server-side tool search or grouping (no SEP accepted), per-result `cache_hint` (SEP-2419 open with no sponsor), and a standard usage `_meta` (no SEP accepted).

---

## 1. How agent harnesses build prompts and use provider caching

### 1.1 Provider rules that decide where content can sit

**Anthropic** (https://platform.claude.com/docs/en/build-with-claude/prompt-caching unless noted):
- **Order and invalidation.** The prefix order is `tools → system → messages`. "Changes at each level invalidate that level and all subsequent levels."
- **Breakpoints.** At most 4 breakpoints; "The lookback window is 20 blocks"; "Cache writes happen only at your breakpoint."
- **Automatic caching.** A single top-level `cache_control` puts the breakpoint on the last cacheable block. It launched 2026-02-19 according to the release notes at https://platform.claude.com/docs/en/release-notes/overview.
- **Prices.** Writes cost 1.25× (5-minute TTL) or 2× (1-hour TTL); reads cost 0.1×.
  - Exceptions: Opus 5.5 reads at 0.05×; Fable 5.1 and Mythos 5.1 at 0.025× (release note 2026-09-01; https://platform.claude.com/docs/en/about-claude/pricing).
  - Examples, in $/MTok as base input / 5-minute write / 1-hour write / read / output:
    - Sonnet 5.5: $2 / $2.50 / $4 / $0.20 / $10.
    - Opus 5.5: $4 / $5 / $8 / $0.20 / $20.
- **Minimum cacheable length.** 512 tokens for Opus 5.5, Sonnet 5.5 and Fable 5.1; 1,024 for Sonnet 5 and Opus 4.8; 4,096 for Haiku 4.5. Below the minimum, nothing is cached and no error is returned.
- **Concurrency.** "For concurrent requests, note that a cache entry only becomes available after the first response begins. If you need cache hits for parallel requests, wait for the first response before sending subsequent requests." (saved copy, L623.)
- **Pre-warming.** Sending `max_tokens: 0` writes the cache, and "Zero output tokens are billed."
- **Isolation.** Per workspace on the Claude API, Claude Platform on AWS and Foundry; per organization on Bedrock and Google Cloud.
- **Rate limits.** "For most Claude models, only uncached input tokens count toward your ITPM" (https://platform.claude.com/docs/en/api/rate-limits).
- **Changing the prompt mid-conversation without a cache miss.**
  - Append a `role: "system"` message.
  - Use the `inline-tools-2026-09-15` `tool_addition` block and "leave `tools` exactly as you first sent it" (https://platform.claude.com/docs/en/build-with-claude/mid-conversation-system-messages).
  - Tool search with `defer_loading`: "The prefix is untouched, so prompt caching is preserved" (https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool).
- **MCP connector.** "only tool calls are currently supported". `mcp_toolset` accepts `cache_control` and `defer_loading`, and the `mcp-client-2026-09-15` beta can pin a server's tool list for the whole conversation (https://platform.claude.com/docs/en/agents-and-tools/mcp-connector).
- **Cache diagnostics** reached general availability on 2026-09-23. Passing `diagnostics.previous_message_id` returns a `cache_miss_reason` of `model_changed`, `system_changed`, `tools_changed`, `messages_changed`, `previous_message_not_found` or `unavailable` (https://platform.claude.com/docs/en/build-with-claude/cache-diagnostics).

**OpenAI** (https://developers.openai.com/api/docs/guides/prompt-caching unless noted):
- **GPT-5.6 and later.**
  - The minimum is 1,024 visible tokens.
  - Breakpoints are implicit or explicit (`prompt_cache_options.mode`, `prompt_cache_breakpoint`).
  - The only TTL value is `30m`.
  - Writes cost 1.25× and reads 0.1× (0.05× on GPT-6.1 Sol).
- **Earlier models.** No write charge. `prompt_cache_retention` is `in_memory` or `24h`; `24h` is the default for non-ZDR organizations.
- **Routing.**
  - "Cached states live on individual machines, where traffic above 15 requests per minute can lead to overflow routing."
  - Routing hashes the initial tokens, "including tool definitions."
  - Before GPT-5.6, `prompt_cache_key` affects routing. From 5.6 on it only separates cache accounting.
- **Tool search.** Deferred tools are "loaded at the end of the model's context window… This allows the model's cache to be preserved" (https://developers.openai.com/api/docs/guides/tools-tool-search).
- **Hosted MCP.** "As long as the `mcp_list_tools` item is present in the context of an API request, the API will not fetch a list of tools from the MCP server again at each turn" (https://developers.openai.com/api/docs/guides/tools-connectors-mcp).

### 1.2 Harness by harness

#### Claude Code (npm `@anthropic-ai/claude-code` 2.1.286, 2026-09-30; binary 2.1.267 inspected)

**a. Tool definitions**
- MCP tools go into Anthropic `tools`.
- **Tool search is on by default.** It turns off for a non-first-party `ANTHROPIC_BASE_URL`, for `ENABLE_TOOL_SEARCH=false`, and for older Vertex models (https://code.claude.com/docs/en/mcp; https://code.claude.com/docs/en/env-vars).
- Deferred stubs are "just the tool name, with `defer_loading: true`… the same stubs are always present in the same order" (https://claude.com/blog/lessons-from-building-claude-code-prompt-caching-is-everything, 2026-04-30).
- "Claude Code keeps the tool list from the conversation's first request for the whole conversation"; a server that connects later supplies deferred definitions (https://code.claude.com/docs/en/prompt-caching).
- Opt-outs: `alwaysLoad: true` per server, `_meta["anthropic/alwaysLoad"]` per tool, and `ENABLE_TOOL_SEARCH=auto[:N]` (default 10% of context) (https://code.claude.com/docs/en/mcp).
- `list_changed` refreshes tools, prompts and resources (same page).

**b. Tool results**
- Results go into `tool_result` in messages.
- A warning appears above 10,000 tokens. The default cap is 25,000 tokens (`MAX_MCP_OUTPUT_TOKENS`), above which the output spills to a file.
- `_meta["anthropic/maxResultSizeChars"]` raises one tool's text limit up to 500,000 characters (https://code.claude.com/docs/en/mcp).

**c. Breakpoints**
- `cache_control` goes "to `system` blocks and to `messages` entries, including `role: "system"` entries appended mid-conversation" (https://code.claude.com/docs/en/llm-gateway-protocol).
- The TTL is 5 minutes on API-key and cloud-provider access. On a subscription within plan usage, the main conversation gets 1 hour.
- Controls: `ENABLE_PROMPT_CACHING_1H`, `CLAUDE_CODE_PROMPT_CACHE_TTL`, `CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL`, `FORCE_PROMPT_CACHING_5M`, `DISABLE_PROMPT_CACHING[_<MODEL>]` (https://code.claude.com/docs/en/prompt-caching; env-vars).

**d. Deferred tools:** yes, by default; see a.

**e. Server-injected context**
- **`instructions`** are truncated to 2,048 characters per server, overridable with `CLAUDE_CODE_MAX_MCP_DESCRIPTION_LENGTH` from v2.1.280 (https://code.claude.com/docs/en/mcp; CHANGELOG).
- The shipped binary renders them as an `mcp_instructions_delta` attachment: a meta *user* message headed `# MCP Server Instructions`. Servers are sorted by name and each is announced once. This is an implementation detail and may be flag-gated.
- The v2.1.70 changelog entry reads "Fixed prompt-cache bust when an MCP server with `instructions` connects after the first turn" (https://github.com/anthropics/claude-code/blob/main/CHANGELOG.md).
- **Resources** arrive as attachments when @-mentioned. **Prompts** run as `/server:prompt` commands, injected as user messages, which is cache-safe (https://code.claude.com/docs/en/mcp; prompt-caching).

**f. Other**
- Subagents do *not* read the parent's cache: "Its first request doesn't read the parent's cache, because the two prefixes differ." Forks do.
- In a workflow fan-out, Claude Code "holds all but the first until the first agent's response begins… so their first requests read the shared prefix". The hold is `CLAUDE_CODE_WORKFLOW_PREFIX_STAGGER_MS`, default 5000 (https://code.claude.com/docs/en/prompt-caching).
- Usage reporting:
  - OTel `claude_code.token.usage` with `type` ∈ {input, output, cacheRead, cacheCreation}, plus `mcp_server.name` attribution (https://code.claude.com/docs/en/monitoring-usage).
  - `-p --output-format json` reports `usage.cache_creation.ephemeral_{5m,1h}_input_tokens`.

#### Claude Agent SDK (TS 0.3.286; Python 0.2.163)

- **Architecture.** The SDK is "a library that runs the Claude Code binary" (https://code.claude.com/docs/en/agent-sdk/overview), so the Claude Code behaviour above applies.
- **Defaults.** MCP schemas are deferred by default, with up to 10,000 tools, and results are capped at 25k tokens before spilling to a file (https://code.claude.com/docs/en/agent-sdk/tool-search; /mcp).
- **Cross-machine reuse.** `excludeDynamicSections: true` moves per-user context into the first user message "so identical configurations share a cache entry across users and machines". TS also offers `SYSTEM_PROMPT_DYNAMIC_BOUNDARY`, which splits a custom prompt into two cached blocks (https://code.claude.com/docs/en/agent-sdk/modifying-system-prompts). **This is the most important knob for a Maidan "fleet prefix" (§5).**
- **Usage.** TS `ModelUsage.{cacheReadInputTokens, cacheCreationInputTokens}` per model, including subagents. Python exposes a `ResultMessage.usage` dict (https://code.claude.com/docs/en/agent-sdk/cost-tracking).
- Instruction placement specific to the SDK is **UNVERIFIED**; it is presumably the same as the CLI.

#### OpenAI Codex CLI (openai/codex@`b44ca87`, 2026-10-01)

**a. Tool definitions**
- MCP tools go into Responses `tools` as `namespace` specs, sorted by name inside each namespace (`codex-rs/core/src/tools/spec_plan.rs`).
- MCP tools are deferred whenever tool search is available, and every model in the bundled catalog sets `supports_search_tool: true` (`codex-rs/models-manager/models.json`; `features/src/lib.rs`).
- To opt out, set `omit_tools_from = ["deferred"]`.
- `tools/list_changed` is only logged (`rmcp-client/src/logging_client_handler.rs`), so the tool set is effectively frozen per connection.

**b. Tool results**
- Results go into `function_call_output`. The default truncation is 10,000 tokens, overridable per tool with `output_token_limit` (https://developers.openai.com/codex/mcp).

**c. Breakpoints**
- `prompt_cache_key` is the session id. Codex sends no `prompt_cache_retention`, `prompt_cache_options` or breakpoints (`core/src/client.rs`).

**d. Tool search**
- Client-executed BM25 search, returning 8 results by default.

**e. Server-injected context**
- **`instructions`** become the MCP namespace `description`, and the server is listed in the `tool_search` description (`codex-mcp/src/rmcp_client.rs` L127, L667–690; `tool_search_spec.rs`). **Both sit in the tools prefix, so editing the instructions busts everything after them.**
- The docs say "Keep the first 512 characters self-contained". I found no 512-character truncation in the code, so that limit is **UNVERIFIED**.
- **Resources** are reachable through `list_mcp_resources` and `read_mcp_resource` tools.
- I found no handling of MCP **prompts** (**UNVERIFIED** beyond a grep).

**f. Usage**
- `cached_input_tokens` and `cache_write_input_tokens` in `exec --json` output and in OTel (`exec/src/exec_events.rs`; `otel/src/events/session_telemetry.rs`).

#### OpenAI Agents SDK (Python 0.22.3 @`28e9f4f`; JS 0.18.0 @`fdaf0a6`)

- **Tool definitions.** Local MCP tools come before the agent's own tools (`src/agents/agent.py`). `list_tools()` runs **every turn** unless `cache_tools_list=True` (`docs/mcp.md`), so any change in the list changes the prefix.
- **`list_changed`** is not handled automatically.
- **Tool results.** No default truncation. `ToolOutputTrimmer` rewrites older turns, which by design defeats prefix reuse.
- **Caching.** Python generates `prompt_cache_key` automatically, stable per run, session or conversation (`run_internal/prompt_cache_key.py`). `prompt_cache_retention` and `prompt_cache_options` are settable.
- **Deferred tools.** `ToolSearchTool` with `defer_loading` covers function tools, namespaces and *hosted* MCP. I found no deferral for local MCP (**UNVERIFIED** absence).
- **Instructions are ignored.** `server_initialize_result` is stored but never read in the prompt path (`src/agents/mcp/server.py`). Prompts and resources are manual.
- **Usage.** `input_tokens_details.cached_tokens` and `cache_write_tokens`, plus `request_usage_entries`.

#### Cursor (closed source)

- **Tool definitions.** Cursor uses "dynamic context discovery for MCP by syncing tool descriptions to a folder"; the agent "only receives a small bit of static context, including names of the tools". The change "reduced total agent tokens by 46.9%" (https://cursor.com/blog/dynamic-context-discovery, 2026-01-06).
- **Tool results.** Long results are written to a file the agent can read (same post).
- **Caching.** A Cursor staff member says roughly 47.5k tokens of tool definitions, system prompt, rules and skills were read from cache across chats in one repo (https://forum.cursor.com/t/151439, 2026-02-23). Breakpoint placement is **UNVERIFIED**.
- **Unknowns.** Handling of `instructions` (**UNVERIFIED**; a question at https://forum.cursor.com/t/77281, 2025-04-09, went unanswered), the request shape, and `list_changed` handling (all **UNVERIFIED**).
- **Usage.** The Admin API returns `tokenUsage{inputTokens, outputTokens, cacheWriteTokens, cacheReadTokens, totalCents}` (https://cursor.com/docs/account/teams/admin-api).

#### Goose (aaif-goose/goose@`bab8ff6`, 2026-09-30, v1.53.0; block/goose redirects here)

**a. Tool definitions**
- Native `tools` named `{extension}__{tool}`, **sorted by name**. The code comment reads: "Stable tool ordering is important for multi session prompt caching" (`crates/goose/src/agents/reply_parts.rs` L302).
- A toolshim fallback renders tools into the system prompt instead.
- `list_changed` invalidates the cache and the list is fetched again (`mcp_client.rs` L217–223).

**b. Tool results**
- Text over 200,000 characters (`GOOSE_MAX_TOOL_RESPONSE_SIZE`) is spilled to a temp file.

**c. Breakpoints**
- System block, last tool ("all tool definitions will be cached as a single prefix") and the last two user messages. 5-minute TTL, with an optional 1-hour TTL (`crates/goose-provider-types/src/formats/anthropic.rs` L514–595).

**d. Deferred tools**
- Code Mode is on by default in goose-cli (`crates/goose-cli/Cargo.toml` L95–101). It exposes three meta-tools (`list_functions`, `get_function_details`, `execute_typescript`) instead of direct tools.
- The Extension Manager can enable and disable extensions.

**e. Server-injected context**
- **Full `instructions` go into the system prompt**, one `## {extension}` section each, sorted by name "for multi session prompt caching" (`crates/goose/src/prompts/system.md`; `prompt_manager.rs` L111).
- The date and time are fixed at hour granularity "so that prompt cache can be used" (`prompt_manager.rs` L189–193).
- The per-turn `<turn-context>` block accepts contributions from platform extensions only.
- Resources are reached through tools.

**f. Usage**
- Cache read, cache write and cost are printed in the CLI (`goose-cli/src/session/output.rs` L1620–1643).
- Auto-compaction triggers at 0.8 of context.

#### OpenHands (OpenHands/software-agent-sdk v1.50.1 @`fad6377`, 2026-09-30; the OpenHands repo is now the front end)

- **Tool definitions.** Native, through LiteLLM, using raw MCP names. I found no sort. `list_changed` adds tools at runtime (`agent/base.py` L890–905).
- **Tool results.** Truncated at 50,000 characters (`llm/message.py` L472–480).
- **Breakpoints.** On the *static* system block (the dynamic block is left unmarked "to enable cross-conversation prompt caching") and on the last user or tool message (`llm/llm.py` L2993–3021).
- **OpenAI caching.** `prompt_cache_key` is the conversation id, and `prompt_cache_retention` defaults to 24h. Gemini is excluded from explicit markers because they would "disable Google's implicit caching" (`model_features.py` L221–223).
- **Filtering.** `filter_tools_regex` only; no tool search.
- **Server context.** I found no use of `instructions`, resources or prompts (grep).
- **Usage.** `TokenUsage.cache_read_tokens` and `cache_write_tokens` (`llm/utils/metrics.py`). The condenser rewrites history after `keep_first=2`.

#### LangGraph / LangChain (langchain-mcp-adapters 0.3.2 @`52a4535`; langchain-anthropic 1.7.5; langgraph-bigtool 0.0.3)

- **Tool definitions.** MCP tools become `StructuredTool`s, bound natively with `bind_tools`. I found no `list_changed` handling.
- **Tool results.** No truncation. `structuredContent` is kept as an artifact the model does not see (`tools.py` L240–283).
- **Breakpoints.** `AnthropicPromptCachingMiddleware` marks the last system block and the last tool, and sets a top-level `cache_control`. Without the middleware there are no breakpoints (`langchain_anthropic/middleware/prompt_caching.py`).
- **Tool selection.**
  - `LLMToolSelectorMiddleware(max_tools=…)`.
  - `ProviderToolSearchMiddleware`, which sets `defer_loading` and injects the provider's tool search.
  - langgraph-bigtool re-binds the selected tools every turn, so (by inference) every new selection busts the prefix (`langgraph_bigtool/graph.py` L79–95).
- **Server context.** `instructions` must be injected manually via `get_server_info()`. The docstring says "the MCP SDK caches only capabilities and discards instructions" (`server_info.py`).
- **Usage.** `usage_metadata.input_token_details.cache_read` and `cache_creation`.

#### CrewAI (crewai 1.15.23 @`fad444d`, 2026-10-01; crewAI-tools is archived and merged into the main repo)

- **Tool definitions.** Native when the model supports function calling; otherwise ReAct text in the prompt (`crew_agent_executor.py` L340–348). MCP schemas are cached for 300 seconds.
- **Breakpoints.** On the system block and on the initial task user message. There is no tool marker and no rolling tail breakpoint (`llms/cache.py`; `anthropic/completion.py`).
- **Filtering.** `StaticToolFilter` and a dynamic `tool_filter`. No tool search.
- **Server context.** No `instructions`. `list_prompts` exists but has no caller.
- **Usage.** `cached_prompt_tokens` and `cache_creation_tokens`.

#### Cline (cline/cline: SDK bundle @`9fe1759`, 2026-09-30; legacy branch @`7761370`)

The stable VSIX ships two code paths and A/B-tests them: the legacy extension for most users and the SDK extension for a rollout that "start[ed] at 1%". The current percentage is **UNVERIFIED**.

**SDK bundle**
- **Tool definitions.** Native, named `server__tool`, unsorted. A tool-list change *restarts the session* (`sdk-mcp-coordinator.ts`).
- **Tool results.** Capped at 8,000 characters. The full text is kept behind a `cline://cache/…` URI (`message-builder.ts` L29–37), and a code comment warns that truncation "invalidates provider prefix caches from the first rewritten block onward".
- **Breakpoints.** **One**, on the last user message (`llms/src/providers/ai-sdk.ts` L386–425).
- **Server context.** Instructions, resources and prompts are not injected.

**Legacy**
- **Tool definitions.** XML prompt variants render an "MCP SERVERS" section in the system prompt, with full tool schemas, resources and prompts (`core/prompts/system-prompt/components/mcp.ts`). Native variants instead use native tools named `uid0mcp0tool`.
- **Breakpoints.** The system block and the last two user messages. Tools are deliberately left unmarked.
- **Tool results.** Capped at 400 KB.
- **Server context.** Instructions are not read.

#### Aider (Aider-AI/aider@`5dc9490`, 2026-05-22; last release v0.86.0, 2025-08-09)

- **No MCP.**
  - A grep of `aider/` for MCP returns nothing.
  - `requirements.in` avoids `litellm[proxy]` because "it installs mcp".
  - Issues #2525 and #3314 are open. PRs #3672, #3937 and #5694 were closed without merging.
- **Breakpoints.** `--cache-prompts` marks system/examples, the repo map or read-only files, and the chat files (`aider/coders/chat_chunks.py` L28–62).
- **Cache warming.** `--cache-keepalive-pings N` sends a `max_tokens=1` ping every 295 seconds.
- Aider is relevant to Maidan only through REST or CLI wrappers.

#### pi (earendil-works/pi@`8ce69e9`, 2026-10-01, v0.99.2; badlogic/pi-mono redirects here)

**History.** On 2025-11-30, Mario Zechner wrote that pi omits MCP on purpose, because servers like "Playwright MCP (21 tools, 13.7k tokens)… dump their entire tool descriptions into your context" (https://mariozechner.at/posts/2025-11-30-pi-coding-agent/). v0.99.0 (2026-09-29) "Added codemode, tool search, and MCP support as built-in extensions" (`packages/coding-agent/CHANGELOG.md` L84).

**a. Tool definitions**
- Each server has an exposure setting: `codemode` (the default; tools are not declared and are reachable from scripts), `deferred` (declared after `tool_search`), `direct`, or `hidden`.
- On Anthropic, tools added later arrive as `defer_loading` plus `tool_addition` blocks: "The request-level list therefore only grows, keeping the cached prefix intact" (`packages/ai/src/api/anthropic-messages.ts` L1139–1146).
- v0.99.2 removed server instructions and tool counts from the codemode and tool_search descriptions "so it no longer changes when MCP servers connect" (CHANGELOG L34).

**b. Tool results**
- Results above 20 KB reach the model with their middle removed; the full text goes to a temp file.

**c. Breakpoints and warming**
- System block, last tool and last message. `cacheRetention: short|long` (long means 1 hour).
- On OpenAI, `prompt_cache_key` is the session id.
- **Cache warming** (`streaming` by default) refreshes at 90% of the TTL, and only when the expected saving is at least $0.05 (`src/core/cache-warmer.ts` L15–31).

**e. Server-injected context**
- An `mcp_servers` system-prompt section has one line per server, sorted by name. The line is the configured description or *the first line of `instructions`*, capped at 250 characters; the whole section is capped at 4,096. Changes are appended as a mid-conversation system message (`src/extensions/mcp/index.ts` L148–207; `agent-session.ts` L1659–1683).

**f. Usage**
- `Usage{input, output, cacheRead, cacheWrite, cacheWrite1h?, cost}`, and a TUI footer showing `R`, `W`, `CH%` and `$` (`packages/ai/src/types.ts` L427–448).

Maidan's docs mention "pi" only as the origin of the renamed `pi.waiter.result/1` envelope (`docs/Result Delivery.md`). `docs/Pi.md` is about Raspberry Pi, not this harness.

### 1.3 Cross-harness table

| Harness | Where MCP tool defs go | Where tool results go (cap) | Cache breakpoints | Deferred tools / tool search | Server `instructions` | Resources / prompts | Usage incl. cache tokens |
|---|---|---|---|---|---|---|---|
| Claude Code | Anthropic `tools`; deferred stubs by default, list frozen at first request | `tool_result` (25k tokens, then file) | system + messages; 5m/1h | **Yes, default on** | Meta user message, ≤2,048 chars, sorted, once | @-mention attachment / `/` commands | OTel cacheRead / cacheCreation; JSON 5m/1h |
| Claude Agent SDK | Same binary | Same | Same, plus `excludeDynamicSections` | Yes, default | Same (SDK-specific: UNVERIFIED) | Same | `ModelUsage.cacheRead/CreationInputTokens` |
| Codex CLI | Responses `namespace`, sorted inside; deferred | `function_call_output` (10k tokens) | `prompt_cache_key`=session only | **Yes**, BM25, 8 results | **Namespace description, in tools prefix** | Resource tools; prompts: none found | `cached_input_tokens`, `cache_write_input_tokens` |
| OpenAI Agents SDK | `tools`, MCP first, re-listed each turn | `function_call_output` (none) | Auto `prompt_cache_key` (Py) | Hosted MCP only | **Ignored** | Manual | `cached_tokens`, `cache_write_tokens` |
| Cursor | Names in context, descriptions in files | Written to file | Not configurable; cross-chat hits (staff) | File-based discovery | UNVERIFIED | "Supported" (placement UNVERIFIED) | Admin API cacheRead / cacheWrite |
| Goose | `tools`, sorted; toolshim → system | Messages (200k chars, then file) | system + last tool + last 2 user msgs | Code Mode (default in CLI) | **Full, in system prompt, sorted** | Via tools / CLI | cache read/write + $ |
| OpenHands SDK | `tools`, raw names, unsorted | Tool msg (50k chars) | Static system + last msg; `prompt_cache_key` | Regex filter only | Ignored | Ignored | cache_read/write + cost |
| LangChain / LangGraph | `bind_tools` | `ToolMessage` (none) | Only with middleware | Selector, provider search, BigTool | Manual | Manual | cache_read / cache_creation |
| CrewAI | Native, or ReAct text | Tool msg / Observation | system + task msg | Static/dynamic filter | Ignored | Ignored | cached / cache_creation |
| Cline SDK | `tools`, unsorted; change → session restart | 8,000 chars + recovery URI | **One** (last user msg) | Enable/disable only | Ignored | Ignored | cacheRead/Write + $ |
| Cline legacy | XML: **schemas in system prompt**; or native | 400 KB | system + last 2 user msgs | Server disable | Ignored | Listed in system prompt | cacheRead/Write |
| Aider | No MCP | n/a | `--cache-prompts` + keepalive | n/a | n/a | n/a | cache write/hit + $ |
| pi 0.99.2 | codemode (default) / deferred / direct; Anthropic `tool_addition` | 20 KB, middle removed | system + last tool + last msg; warming | **Yes** | **First line, ≤250 chars**, appended on change | Resource tools; prompts UNVERIFIED | cacheRead / cacheWrite / cacheWrite1h, CH% |

Sources: §1.2. The harness pages and code paths cited there.

### 1.4 What this means for where Maidan's stable context can sit

The prompt has four layers that Maidan can reach.

**L0, tools prefix.**
- **What lands here:** Maidan's tool definitions. Codex also puts Maidan's `instructions` here.
- **How stable it is:** most stable, and shared by every turn.
- **Its weakness:** a change invalidates everything after it.

**L1, system prompt.**
- **What lands here:** Maidan's `instructions`, but only in Goose (in full) and pi (one line).

**L2, conversation.**
- **What lands here:** tool results (context packs, search hits), resources and prompts. Claude Code also delivers `instructions` here.
- **How it caches:** once a result is appended, it is cached for all *later* turns of that agent, because the harness appends and the prefix grows.
- **What breaks it:** compaction and trimming (OpenHands condenser, Goose auto-compaction, Agents SDK `ToolOutputTrimmer`, Cline truncation), which rewrite this layer.

**Cross-agent sharing.**
- Requires L0 + L1 + the L2 position before Maidan's content to be byte-identical across agents, the same workspace or organization, and recency.
- Realistic only for fleets Maidan configures: same harness build, `excludeDynamicSections` in the Agent SDK, the same named capability set, and the same API workspace.

Recommendations that follow directly:

- **H1. Treat `tools/list` bytes as a cache key.**
  - What already holds: the order is a fixed `Vec` literal, and the descriptions are static. `catalog.rs` has no runtime interpolation outside its tests.
  - What breaks sharing: `catalog_for(auth)` filters by capability (`tools/mod.rs` L59–72), so agents with different atomic capabilities get *different* tool prefixes and therefore different caches.
  - The fix: put fleets on one named capability set (e.g. `maidan.agent.worker`).
  - The guard: add a contract test asserting byte-identical `tools/list` across replicas and across two tenants with the same capability set. Both are SHOULD-level spec expectations (§2).
- **H2. Ship a small default surface for harnesses without tool search.**
  - Who needs it: Goose (without Code Mode), OpenHands, Cline, CrewAI, LangChain and the Agents SDK resend about 27.6k tokens of Maidan tool definitions on every request.
  - What to ship: a server-side profile, the seven-tool hero loop (≈2k tokens), selected by token or capability set. That is better than relying on client-side filters (§5, idea 9).
  - For harnesses that do search: use consistent name prefixes and keyword-rich descriptions. Anthropic advises "Keep your 3–5 most frequently used tools non-deferred" (tool-search-tool docs). In Claude Code, `_meta["anthropic/alwaysLoad"]` does that per tool.
- **H3. Write `instructions` for the worst placement.**
  - Make the first line a self-contained summary of at most 250 characters (pi).
  - Keep the whole text under 2,048 characters (Claude Code). It is 1,305 today.
  - Keep it byte-stable per release, because Codex puts it in the tools prefix.
  - Serve it from `server/discover` with a long `ttlMs`.
  - Never put per-room or per-tenant text in it.
- **H4. Size tool results for the smallest cap.**
  - The scoped pack measured in `docs/Benchmark.md` is 19,802 bytes. That exceeds Cline SDK's 8,000-character cap and approaches pi's 20 KB "middle removed" threshold. Both harnesses then cut it arbitrarily instead of using Maidan's own auditable `elision`.
  - Default `token_budget` from `clientInfo` (the 2026 `_meta` carries it on every request), and prefer `ResourceLink`s for bulk content.
- **H5. Keep L2 append-only.** On a re-read, return "what changed since the pack you last received" rather than a new full pack (§5, idea 5). This follows the advice to "Make your context append-only" (Manus) and avoids the rewrite costs measured in arXiv 2607.12161 (§3).
- **H6. Pull usage from where it already exists** (Claude Code OTel, Codex JSON, harness `Usage` objects) instead of trusting manual `report_usage` calls (§4.5).

---
## 2. The MCP spec (2026-07-28 and SEPs in flight) and A2A: caching and stability

Spec sources: modelcontextprotocol/modelcontextprotocol@`046fa30` (2026-09-28); typescript-sdk@`433eb41` (2026-09-30); python-sdk@`d639cf7` (2026-09-30); conformance@`7169291` (2026-09-11); a2aproject/A2A@`1ae57a6` (2026-09-29), with the v1.0.1 spec and proto.

### 2.1 Summary table

| Feature | Status | Semantics (brief quote) | Client support | What it lets Maidan do |
|---|---|---|---|---|
| `ttlMs` / `cacheScope` (`CacheableResult`) | **Required** in 2026-07-28 (SEP-2549, PR merged 2026-05-15). Covers `server/discover`, `tools/list`, `prompts/list`, `resources/list`, `resources/templates/list` and `resources/read` | "analogous to HTTP Cache-Control max-age". `"private"`: "Caches MUST NOT be shared across authorization contexts" | TS, Python, Rust (`rmcp` `service/client/cache.rs`), Go and C# SDKs. TS and Python clients cache by default with a 24 h cap. Conformance checks `sep-2549-*` record a missing field as an error | Become conformant. Let SDK clients and subagents reuse cached lists. Push invalidation instead of having clients poll |
| Deterministic `tools/list` order | SHOULD (PR #2516, merged 2026-04-13) | "Deterministic ordering enables clients to reliably cache the tool list and improves LLM prompt cache hit rates" (server/tools) | Conformance WARNING `tools-list-deterministic-order` (negative fixture `tools-list-rotated-order.ts`) | Keep the tools prefix byte-stable across turns, reconnects and agents |
| Lists must not vary per connection | MUST NOT (SEP-2567, merged 2026-05-07) | Lists "MUST NOT vary per-connection" and "MAY vary by the authorization presented" | n/a | Maidan's capability-filtered lists are legal but must be `"private"` |
| `list_changed` without sessions | `subscriptions/listen` (SEP-2575, merged 2026-05-11) | The server sends `notifications/tools/list_changed` to streams opened with `toolsListChanged: true`, and "MUST NOT send notification types the client has not explicitly requested" | SDK caches evict on these notifications | Use a long TTL plus push-on-change, e.g. on deploy or grant change |
| Resource subscriptions | `resourceSubscriptions: [uris]` on `subscriptions/listen`. `resources/subscribe` removed | `notifications/resources/updated {uri}` | SDK caches evict cached bodies | Push room and thread changes. Maidan still implements `resources/subscribe` (`server.rs` L508), which is the 2025 form |
| ETags / conditional reads | **Not in 2026-07-28.** Roadmap (updated 2026-08-22): "extend our caching approach to support ETags… versioning the results of primitives, in particular tool calls" | — | none | Use content-addressed URIs (`maidan://artifacts/{sha256}`) as de facto ETags until then |
| `annotations.lastModified`, `Resource.size` | Core | `size` "can be used by Hosts to… estimate context window usage" | Hosts that use it: UNVERIFIED | Let hosts budget context before a read |
| `instructions` | Now in `DiscoverResult`, which is cacheable | "by including it in a system prompt… should not duplicate information already in tool descriptions" (schema.ts L688–696) | Host-dependent. 2025-11-03 blog: "the exact way that the MCP host uses server instructions is up to the implementer" | Keep them short and pinned to the version. See H3 |
| Skills extension `io.modelcontextprotocol/skills` | **Final**, Extensions Track (SEP-2640, merged 2026-09-13) | Each file listed with `{uri, digest: "sha256:…", size}`. Hosts "MUST NOT retrieve a skill's files ahead of need" and "SHOULD instead cache what they do retrieve". A matching digest means the file "can be served without fetching it again" | Extension client matrix: mcpc full; ChatGPT, fast-agent and MCP Inspector partial | Serve Maidan workflow guides and room digests as lazily loaded, content-addressed files |
| Progressive discovery, tool search, groups | **No accepted SEP.** SEP-2084 rejected 2026-02-05; SEP-1300 rejected; SEP-1821 dormant (closed 2026-06-13); SEP-1576 "token bloat" dormant (closed 2026-06-26); SEP-2053 Server Variants closed 2026-09-23 pending the WG. The Primitive Grouping IG is "Active Exploration" (progressive-disclosure-wg@`2505387`, 2026-09-07) | Client best practice: host-side `search_tools`, and "Append newly discovered definitions after the cache breakpoint rather than re-sorting the `tools` array" (`docs/docs/2026-07-28/develop/clients/client-best-practices.mdx`, PR #2582) | Done in hosts: Anthropic `defer_loading`, OpenAI `tool_search`, Claude Code, Codex, pi | Optimize names and descriptions for host-side search. A server-side profile or variant is Maidan's own choice |
| `cache_hint` on `CallToolResult._meta` | SEP-2419, open proposal with no sponsor (2026-03-18 to 2026-09-10) | `"no-cache"` / `"cache"` per tool result | None known (UNVERIFIED) | Wait |
| Token or cost `_meta` | No SEP accepted. SEP-2448 telemetry closed 2026-09-22; #3229 closed 2026-08-23 | `_meta` allows vendor prefixes and reserves `io.modelcontextprotocol/` | none | Only a vendor-prefixed key, which nobody reads |
| A2A Agent Card HTTP caching | A2A v1.0.0 (2026-03-12) and v1.0.1 (2026-05-28), §8.6 | Server "SHOULD" send `Cache-Control` `max-age` and an `ETag` "derived from the Agent Card's `version` field or a hash". Clients "SHOULD use conditional requests" | Issue #1478 (closed 2026-03-12): "None of the five official A2A SDKs… implement Agent Card caching" | Answer pollers with 304. Keep the card byte-stable |
| A2A `contextId`, `referenceTaskIds`, `historyLength`, `includeArtifacts` | v1.0.1 §3.4.1, §3.4.3, §3.2.4, §3.1.4 | "Agents MAY use the `contextId` to maintain… LLM context across multiple interactions". `historyLength` "0: No history should be returned". `includeArtifacts` "Defaults to false to reduce payload size" | Core spec | Use `contextId` as a cache-affinity key. Pass references instead of transcripts. Keep status polls lean |
| A2A extensions | §4.6, §3.2.6 (`A2A-Extensions` header) | URI-keyed `metadata`. A `required` extension the client lacks returns `ExtensionSupportRequiredError` (-32008) | The a2a-samples traceability extension v1 (@`6603ba3`, 2026-08-04) has `int64 cost` and `int64 total_tokens`. #2121 (cost and budget propagation, open since 2026-08-09) | Publish an optional PayerStamp usage extension per task |

### 2.2 What each item lets Maidan do

1. **Conformance first.** Implement `server/discover`, which carries `instructions` and capabilities and is itself cacheable. Add `ttlMs` and `cacheScope` to the six results. The minimum conformant answer is `ttlMs: 0, cacheScope: "private"`, which both SDKs emit by default and which the Python docs call "always safe and always conformant".
2. **Positive TTLs where the data is immutable or changes only on deploy.**
   - `tools/list` changes only when Maidan is deployed or a grant changes. The spec's examples use 300,000 ms for lists and 3,600,000 ms for discover; clients cap at 24 h.
   - `resources/read` of `maidan://artifacts/{sha256}` is immutable by construction, so it can take the maximum TTL.
   - Mutable resources (threads, channels) get a short TTL plus `resourceSubscriptions`.
3. **`cacheScope` must follow tenancy.** Maidan's lists are filtered by capability, so they are `"private"`. `"public"` is safe only for content identical for everyone, such as an unfiltered catalog or a public discover document.
   - The spec says servers "MUST NOT rely on `cacheScope` alone to prevent unauthorized access".
   - Open issue #3207 (2026-08-06) argues that `public` enables cross-user poisoning at shared gateways.
   - This is the same failure shape as Maidan's 2026-09-25 optional-filter leaks, so write a two-tenant test for it.
4. **Subagent economics.** SEP-2567's motivation is exactly the coordination case: per-session lists forced "`O(subagents × servers)` calls to `tools/list`… Under this proposal the same workload is `O(servers)`… a subagent can then inherit its parent's cached lists at zero cost" (`seps/2567-sessionless-mcp.md`).
5. **Skills as a delivery vehicle.** Content-addressed digests in the Skills extension match Maidan's artifact store one-to-one. A "room skill" (glossary, accepted decisions, workflow guide) can be fetched lazily, cached by digest, and re-fetched only when the digest changes.
6. **What not to build yet:** server-side tool search (no SEP; maintainers prefer client-side), `cache_hint` (no sponsor), and a usage `_meta` key (no reader).

### 2.3 Provider-hosted MCP clients

- **Anthropic MCP connector:**
  - Tools only.
  - "You don't control tool order within an MCP toolset, so place the breakpoint on the `mcp_toolset` entry itself" (prompt-caching docs). Maidan's server-side order therefore *is* the cached order.
  - The `mcp-client-2026-09-15` beta records and pins the tool list, so mid-conversation tool changes do not reach the model.
  - Whether the connector speaks 2026-07-28 or honors `ttlMs` is **UNVERIFIED**.
- **OpenAI Responses remote MCP:**
  - The `mcp_list_tools` item stays in context.
  - `allowed_tools` imports a subset.
  - `defer_loading` on the MCP tool (openai-python 3.22.1 `types/responses/tool_param.py`). For a deferred server "the model sees only the namespace or server name and description at the beginning."
  - Implication: Maidan's `server_description` and its tool descriptions are what the model searches over.

---

## 3. Who already pitches LLM cost savings, and how

The unit of every claim below is a **request** or a **token**. None is a completed task.

### 3.1 Gateways, proxies and routers

| Product | Claim (verbatim) | Evidence | Weakness |
|---|---|---|---|
| Helicone | Caching "eliminating redundant API calls and reducing both latency and costs"; provider prompt caching "(up to 90% savings)" (docs.helicone.ai/features/advanced-usage/caching) | None of its own | Exact match on URL, body and headers: "Any change in these components creates a new cache entry". Sessions group calls but record no outcome. Joined Mintlify; services in "maintenance mode" (helicone.ai/blog/joining-mintlify, 2026-03-03) |
| Portkey | "serve requests up to **20x faster** and cheaper"; "~20% cache hit rate at 99% accuracy for Q&A (or RAG)" (portkey.ai/blog/reducing-llm-costs-and-latency-semantic-cache, 2023-07-11) | Internal early tests | Semantic cache only "under 8,191 tokens and ≤4 messages"; "The **system prompt is ignored**", so two different roles can share an answer; Enterprise only (portkey.ai/docs/product/ai-gateway/cache-simple-and-semantic) |
| LiteLLM | "stores and reuses LLM responses to save costs"; "Track spend for keys, users, and teams" | None numeric | **Its own docs:** semantic caching "goes badly wrong on agentic traffic… the client replays a stale response, which typically shows up as an agent repeating the same tool call over and over. Raising similarity_threshold does not reliably fix this" (docs.litellm.ai/docs/proxy/caching_semantic) |
| OpenRouter | "To maximize cache hit rates, OpenRouter uses provider sticky routing" (openrouter.ai/docs/guides/best-practices/prompt-caching) | Mechanism only | "Sticky sessions expire after **10 minutes** of inactivity". A conversation is identified by hashing the first system and first non-system message, or by a caller `session_id`. Auto Router "can pick a different model on every turn" |
| Cloudflare AI Gateway | "Minimize the number of paid requests made to your AI provider" (developers.cloudflare.com/ai-gateway/features/caching, updated 2026-09-30) | None | "applies only to identical requests" |
| Kong AI Gateway | Prompt compressor keeps "80% of the intended semantic meaning… up to 5x cost reduction" (Kong press release, 2025-07-15) | None shown | Concedes about 20% loss of meaning. Rewriting each request also breaks the provider prefix cache |
| Vercel AI Gateway | "no markup… on tokens"; `caching:'auto'`: "Multi-turn and agentic traffic caches well" (vercel.com/docs/ai-gateway/models-and-providers/automatic-caching, 2026-09-11) | Arithmetic on the 1 h TTL | Inserts breakpoints for some providers only. Cannot make two agents' prefixes identical |
| TensorZero | "cheaper models" via a data flywheel. An *episode* is "a sequence of inferences associated with a common downstream outcome" (tensorzero.com/docs/gateway/guides/episodes) | Runnable examples | **Closest to outcome-aware**, but the outcome is whatever the app posts. No assignment, claim or dedup |
| RouteLLM | "reduce costs by up to 85% while maintaining 95% GPT-4 performance" (arXiv 2406.18665, 2024-06-26) | Open benchmarks | Single-turn, binary routing, 2024 models, and a 5% quality loss |
| Not Diamond | "Cost savings 20% +" (notdiamond.ai) | Testimonials and a calculator | No public method. Switching models per input forfeits prefix caches (inference) |

### 3.2 Semantic and response caches

| Product / paper | Claim | Evidence | Weakness |
|---|---|---|---|
| GPTCache | "Slash Your LLM API Costs by 10x 💰" (github.com/zilliztech/GPTCache) | None | Last release 0.1.44 (2024-08-01); "we no longer add support for new API or models" |
| Redis LangCache | "Cut token usage and API bills by up to 70%" (redis.io/blog/langcache-public-preview, 2025-09-04, updated 2026-08-13) | One testimonial: "70% cache hit rate, which saves 70% of our LLM spend" | Treats hit rate as savings. Scoped to users, apps or sessions, not tasks |
| vCache (arXiv 2502.03771, 2025-02-06) | "static thresholds do not give formal correctness guarantees"; "up to 12.5× higher cache hit and 26× lower error rates" | Four benchmarks | Implies that fixed thresholds as shipped have unbounded error |
| Agentic Plan Caching (arXiv 2506.14852, 2025-06-17) | Context and semantic caching are "insufficient for agent applications"; "reduce costs by 50.31%" | Several agent apps | Reuse is drawn "from **completed** agent executions", so it needs to know completion. That is a coordinator's data |
| Auditing Prompt Caching (arXiv 2502.07776) | "global cache sharing across users in seven API providers" | Timing audits | Cross-agent sharing must stay inside a tenant. Providers now isolate (Anthropic per workspace; OpenAI per organization) |

### 3.3 Memory layers

| Product | Claim | Evidence | Weakness |
|---|---|---|---|
| Mem0 (arXiv 2504.19413, 2025-04-28) | "91% lower p95 latency and saves more than 90% token cost" | LOCOMO; 1,764 vs 26,031 context tokens | **Full context scored higher** (72.90 J vs 66.88 / 68.44). "Token cost" counts query-time context only, not extraction calls. The README's 2026 scores "reflect Mem0's managed platform, which includes proprietary optimizations" |
| Zep (arXiv 2501.13956, 2025-01-20) | DMR "94.8% vs 93.4%"; LongMemEval "reducing response latency by 90%" | DMR, LongMemEval_s | Full-conversation baseline 94.4%. DMR is "easily fitting within current LLM context windows". Graph-build cost not reported |
| Letta sleep-time compute (arXiv 2504.13171, 2025-04-17) | "decrease the average cost per query by 2.5x" | Synthetic stateful GSM / AIME | Assumes test-time tokens cost 10× sleep-time tokens and 10 queries per context. "at higher budgets, standard test-time scaling is better" |
| LOCOMO dispute | Mem0 scored Zep at 65.99; Zep's rebuttal (2025-05-06) claimed 84%, corrected to 75.14% (2025-05-12); getzep/zep-papers issue #5 | — | Letta: a filesystem agent got "74.0% accuracy on LoCoMo", above Mem0's 68.5% (letta.com/blog/benchmarking-ai-agent-memory, 2025-08-12) |

What memory layers do not model:
- **No done state.** Mem0 extraction is "ADD-only… nothing is overwritten".
- **No claim or lease primitive** in Zep.
- **Letta shared memory is Git:** "The agent must commit and push its changes."

### 3.4 Context engineering: writing, products and research

| Source | Claim | Note |
|---|---|---|
| Anthropic, "Effective context engineering" (2025-09-29) | "the smallest possible set of high-signal tokens"; subagent summaries "often 1,000-2,000 tokens" | Qualitative |
| Anthropic, "Code execution with MCP" (2025-11-04) | "150,000 tokens to 2,000 tokens… 98.7%" | One illustrative example, not a benchmark |
| Anthropic tool search docs | "Tool search typically reduces this by over 85 percent" | Covers tool-definition overhead only |
| Cloudflare Code Mode (2025-09-26; 2026-02-20) | "reduces the number of input tokens used by 99.9%" | Covers tool-definition footprint only |
| Manus (2025-07-18) | "the KV-cache hit rate is the single most important metric for a production-stage AI agent"; input to output "around 100:1"; "Make your context append-only"; "Mask, Don't Remove" | Single-agent guidance; no hit-rate data |
| Cognition, "Don't Build Multi-Agents" (2025-06-12) | "Share context, and share full agent traces" | Thought experiment |
| Claude Code fan-out stagger | First requests "read the shared prefix instead of each processing it uncached" (code.claude.com/docs/en/prompt-caching) | One machine and directory; no figure published |
| Compresr (YC W26) | "~90% Bill cut vs. sending the full context" (compresr.ai) | Baseline is uncached full context. Query-conditioned rewrites forfeit the prefix cache |
| The Token Company (YC W26) | "compression typically removes 10-50% with no measurable accuracy loss" (thetokencompany.com/agents.md) | Advises compressing resent history, which changes the prefix every turn |
| LLMLingua (arXiv 2310.05736) | "up to 20x compression with little performance loss" | 2023 benchmarks; per-request rewrites |
| "Don't Break the Cache" (arXiv 2601.06007, 2026-01-09) | "prompt caching reduces API costs by 41-80%" | Single-agent; n=40 sessions per condition |
| "Token Reduction Is Not Cost Reduction" (arXiv 2607.12161, v5 2026-08-12) | "reduced delivered tool-output tokens by 38.4% but increased billed cost by 6.8%"; Pearson r = 0.15; evaluate "cost per successful task" | **The methodological template for §4** |

### 3.5 Multi-agent frameworks and research on token cost

- **No framework claims multi-agent token savings.**
  - Agno: "~10,000x faster than LangGraph" instantiation, a Python microbenchmark (github.com/agno-agi/agno README @`29830dc`, 2025-04-08). The current README has no performance claim.
  - smolagents: "30% fewer steps (thus 30% fewer LLM calls)", citing CodeAct (arXiv 2402.01030) "up to 30% fewer actions". Steps are not tokens.
  - CrewAI: "5.76x faster", wall-clock on one QA notebook (README @`84a4d47`, 2026-04-21; removed by 2026-08).
  - MetaGPT uses *more* total tokens than ChatDev (arXiv 2308.00352).
- **Research:**
  - Anthropic's multi-agent research system: "about 15× more tokens than chats"; token usage "explains 80% of the variance"; "Without detailed task descriptions, agents duplicate work" (2025-06-13).
  - MAST (arXiv 2503.13657): step repetition is 15.7% of failure modes, and failing to recognize task completion is 12.4%.
  - "How Do AI Agents Spend Your Money?" (arXiv 2604.22750, v2 2026-04-29): runs of the same task "differ by up to 30x"; "cache reads dominate both raw token volume and dollar cost"; about 50% of actions repeat on the same file for costlier models.
  - AgentPrune, AgentDropout, Optima and S²-MAD (arXiv 2410.02506, 2503.18891, 2410.08115, 2502.04790) cut 21–94% of tokens against deliberately dense topologies, and assume one controller owns the whole message graph.
  - KVCOMM, KVFlow and DroidSpeak (arXiv 2510.12872, 2507.07400, 2411.02820) report "over 70% reuse rate" and "up to 2.19×" speedups, but **require a self-hosted engine with KV access**.
- **Framework state is in-process.**
  - LangGraph `InMemorySaver` checkpoints are lost "When the process restarts".
  - The Agents SDK says "The context object is not sent to the LLM. It is purely a local object."
  - So duplicated work across processes, teams or harnesses is invisible to every framework.

### 3.6 Agent-coordination products

| Product | Cost statement | Gap |
|---|---|---|
| Claude Code agent teams (code.claude.com/docs/en/agent-teams; /costs) | "use significantly more tokens than a single session"; "approximately 7x more tokens… in plan mode" | "Two teammates editing the same file leads to overwrites" |
| OpenAI Codex subagents (developers.openai.com/codex/subagents) | "consume more tokens than comparable single-agent runs" | Edit conflicts |
| Cursor subagents (cursor.com/docs/subagents) | "five subagents in parallel uses roughly five times the tokens" | "each subagent gathers its own context" |
| Factory (factory.ai/news/compressing-context, 2025-07-21; /evaluating-compression, 2025-12-16) | "minimize tokens per task, not per request… avoid repeated work" | Applies to one agent's compression, not dedup across agents |
| Devin | ACU = "a normalized measure of the computing resources… to complete a task"; $2.25 per ACU on the archived 2025-06-03 pricing page | No coordination saving claimed |
| MCP Agent Mail (github.com/Dicklesworthstone/mcp_agent_mail) | "Keeps communication out of your token budget"; `unread_only` "cuts token-burn for polling agents" | Leases are "advisory"; no measurements |
| Beads / Gas Town (Yegge) | "Atomically claim a task"; Gas Town "is a cash guzzler" (2026-01-01) | No savings claim |
| AgentOps, CrewAI AMP | "Track spending Across multiple agents" | Measures spend; does not reduce it |
| Letta, Bedrock AgentCore, Gemini Enterprise, Coral | AgentCore: "CPU scales to zero during I/O wait" | Compute saving, not tokens |
| Small vendors (AgentID "up to 65%", Fast.io "45%") | No method | **UNVERIFIED** |

**Does anyone claim savings from coordination itself?** No major vendor does, and every major vendor documents a cost *increase*. The existing mechanisms are Claude Code's sibling stagger (one machine), Anthropic's references-not-copies pattern, Agent Mail's `unread_only`, and Beads' atomic claims, all qualitative. **Nobody has published a baseline-controlled measurement of coordinated versus uncoordinated parallel agents.**

### 3.7 What positioning is open to a coordination layer

Each position names what competitors structurally cannot see.

- **P1. Dollars per completed task.**
  - What competitors see: gateways see keys, users, teams and caller-defined sessions; memory layers have no done state. TensorZero has an outcome hook but no assignment.
  - What Maidan sees: claims, leases, results, reviews and the `report_usage` ledger, all keyed by thread. So it can report cost per done thread, cost of failed or abandoned claims, and spend after a lease expired. MAST's "not recognizing task completion" failure mode is spend that only a completion-aware layer can label.
  - Supporting voices: Factory ("tokens per task, not per request") and arXiv 2607.12161 ("cost per successful task").
  - Integration rather than competition: pass the Maidan thread id as OpenRouter `session_id`, `Helicone-Session-Id`, TensorZero `episode_id` or a LiteLLM tag, so gateway spend joins to Maidan outcomes.
- **P2. Duplicate-work prevention, priced from the ledger.**
  - A gateway cannot tell that two different prompts are the same task.
  - A fenced claim refused because the task is already held is a countable counterfactual, and the holder's ledger cost prices it.
  - Evidence the waste exists: Anthropic ("agents duplicate work"), MAST (15.7% step repetition), arXiv 2604.22750 (about 50% repeated same-file actions).
- **P3. Cache-shaped shared context.**
  - Maidan's content-addressed packs and snapshots give byte-identical bytes to every claimant.
  - A coordinator knows who claims next, so it can choose the TTL and pre-warm.
  - The cache-read and cache-write columns give a cross-agent hit rate per pack.
  - **Honest constraint (§1.4):** this pays only for homogeneous fleets, or for custom agents that put the pack first.
- **P4. Who-read-what deltas.** Read receipts per snapshot make "changes since snapshot X" a first-class response. That is append-only and therefore cache-friendly. Memory layers retrieve by user or agent, not by assignment.
- **P5. Exact reuse of completed results.**
  - Key the result by task-spec hash plus input-snapshot hash plus model class, after verified completion.
  - Zero false-hit risk by construction, unlike semantic caches (LiteLLM's own warning; vCache).
- **P6. Be the honest benchmark.**
  - The field has early tests (Portkey), one testimonial (Redis), a calculator (Not Diamond), a public LOCOMO dispute, and no controlled coordination measurement at all.
  - An open, pre-registered harness that can report a null result is an uncontested claim.

**Positions to avoid, because primary sources refute them:**
- "Multi-agent is cheaper": refuted by the 15×, about 7× and about 5× figures from Anthropic, Claude Code and Cursor.
- "X% fewer tokens": arXiv 2607.12161 found r = 0.15 between token reduction and cost reduction.
- "Reuse similar answers": LiteLLM's own docs say it breaks agents.

**The defensible frame:** *if you run more than one agent, stop paying twice, and know what each finished task cost.*

---
## 4. Credible measurement

### 4.1 How agent cost is benchmarked today

- **SWE-bench Verified, bash-only (mini-SWE-agent).**
  - `leaderboards.json` (github.com/SWE-bench/swe-bench.github.io, last commit 2026-09-01) carries `cost`, `instance_cost`, `instance_calls`, and `per_instance_details{cost, resolved}` for 44 of 180 entries.
  - Cost per resolved instance (arithmetic from those per-instance rows):
    - Claude 4.5 Opus (high): $376.95 / 384 = **$0.982** (median per instance $0.570, p90 $1.434).
    - Gemini 3 Flash: $0.470.
    - MiniMax M2.5: $0.097.
  - The costs come from `litellm.cost_calculator.completion_cost`, so they inherit litellm's price table. Whether each entry ran with caching on is **UNVERIFIED**.
- **Aider polyglot.** "Percent correct" and "Cost" per run over 225 exercises (`aider/website/_data/polyglot_leaderboard.yml`; latest rows 2025-10-03).
- **Terminal-Bench 4.0** (tbench.ai/leaderboard, rows from 2026-09).
  - Publishes `total_cost_usd`, `cached_input_tokens`, `uncached_input_tokens`, `n_trials`, pass@2..5 and 95% confidence intervals.
  - The top row ($3,267.18) reproduces exactly only if "uncached" actually means *total* input. That reading implies a 95.85% hit rate and no write premium (arithmetic; the field meaning is **UNVERIFIED**).
- **HAL** (hal.cs.princeton.edu; arXiv 2510.11977, 2025-10-13).
  - "cost-aware, and third-party": 21,730 rollouts, accuracy-versus-cost Pareto frontiers.
  - It admits "our cost calculations don't yet account for cache hits", single runs "without statistical validation", and "Providers swap model weights behind stable endpoints".
  - Updates are now paused.
- **Others.**
  - OpenHands Index: "Average Cost" (unit **UNVERIFIED**).
  - METR "expenditure horizon" (2026-07-21).
  - τ-bench pass^k (arXiv 2406.12045): the chance that all k trials succeed, versus pass@k.
- **Methodology papers.**
  - *AI Agents That Matter* (arXiv 2407.01502, 2024-07-01). Evaluations "must be cost-controlled". Report token counts so that "anyone… can instantly recalculate the cost using current prices". Agents are "rarely accompanied by error bars".
  - *Token Reduction Is Not Cost Reduction* (arXiv 2607.12161, v5 2026-08-12).
    - Design: blocks of task × model × effort × repetition, arms in randomized order, billed `total_cost_usd` cross-checked against usage fields.
    - Metric: cost per success, CPS = Σcost / |successes|. Statistics: task-clustered bootstrap with 10,000 resamples.
    - Repeated runs of a task are correlated (ICC 0.37–0.55), so the effective sample is about 38–45 tasks despite 712 runs per arm.
    - Cache creation and reads were about 87% of reconstructed cost.
    - It names "prompt-cache carryover" between arms a "first-order threat".
  - *TraceLab* (arXiv 2606.30560). Hit rate = P/(P+A), with P = cache reads and A = input + cache creation. Global hit rate 95.7%. Raising the timeout from 1 minute to 1 hour lifts the achievable hit rate "from 85.4% to 98.6%".
  - *Don't Break the Cache* (arXiv 2601.06007). Caching cut costs 41–80%. "naive full-context caching… can paradoxically increase latency."

### 4.2 Accounting pitfalls to design against

1. **Provider field semantics differ.**
   - Anthropic `input_tokens` excludes cache tokens: `total = cache_read + cache_creation + input`.
   - OpenAI `input_tokens` *includes* `cached_tokens` and `cache_write_tokens`.
   - Mistral's `prompt_tokens` includes cached tokens.
   - DeepSeek splits input into hit and miss.
   - Sources: provider pages in §1.1, https://docs.mistral.ai/studio-api/conversations/advanced/prompt-caching, https://api-docs.deepseek.com/guides/kv_cache.
2. **Write premiums are real.** Anthropic charges 1.25× or 2×; OpenAI GPT-5.6 and later charge 1.25×; older OpenAI models charge nothing for writes.
3. **Cached thinking blocks count as input** (Anthropic).
4. **Measure cost per success, not per attempt.** Repeated runs are not independent (ICC above).
5. **Variance.** The same task can vary up to 30× (arXiv 2604.22750).
6. **pass@k and pass^k answer different questions.** A coordinator needs pass^k.
7. **Prices drift.**
   - Gemini 3.8 Flash prices double on 2027-01-01.
   - GPT-5.6 Sol's promotional pricing runs "at least through November 21, 2026".
   - DeepSeek's off-peak rate is half the peak rate.
   - Anthropic US-only `inference_geo` adds a 1.1× multiplier.
   - Sources: https://ai.google.dev/gemini-api/docs/pricing, https://developers.openai.com/api/docs/pricing, https://api-docs.deepseek.com/quick_start/pricing.
8. **Caching can silently not happen.** Prefixes below each model's minimum (512 to 4,096 tokens) are never cached. Older OpenAI models round cached counts to multiples of 128, Mistral to 64.
9. **Batch cache hits are best-effort.** Anthropic reports "cache hit rates ranging from 30% to 98%" in batches. Batch and cache discounts stack differently by model (Gemini 3.1 Pro: batch caching is "Same as Standard").
10. **Cache carryover across arms** contaminates comparisons. Silent endpoint model swaps are a second hazard.

### 4.3 Proposed benchmark: **Maidan Context-Economics Benchmark v1 (MCEB-1)**

Protocol version 1.0, dated 2026-10-01. Each run is stamped with run dates and a price-snapshot date.

**Claim under test.** For N agents working a shared backlog, Maidan's coordination and cache-stable context lower **dollars per completed task (CPS)** compared with an honest baseline that already uses provider prompt caching. The success rate must stay within a pre-registered non-inferiority margin.

**Mechanism hypotheses** (each tested separately; any of them may come out null):
- **H1:** fewer cache-*write* tokens per success, because the fleet writes the prefix once.
- **H2:** fewer duplicated or abandoned attempts per success, because of claims and dedup.
- **H3:** fewer turns per success, because packs and digests give better context.
- **H4:** token hit rate changes little, because the baseline is already about 95%. **Expected null; report it.**

**Seeded workspace.**
- A Maidan workspace imported from a **signed workspace export**, using the existing `token:admin` export / verify / import feature. The export's sha256 is published.
- The corpus is a frozen repository snapshot plus documents, glossary and accepted decisions, referenced by content hash.
- **Task set:**
  - K Maidan-native backlog threads with deterministic graders. For example: patch tasks graded by tests, doc tasks graded by exact-match checks, and review tasks graded against seeded defects.
  - An external anchor subset of SWE-bench Verified run through mini-swe-agent, so per-instance costs can be compared against the public leaderboard data.
  - Dev and frozen-holdout splits.
- **Shared context by design.** Tasks share a large common context: the repo, the glossary, and decisions above the 512- and 1,024-token minimums. A *below-minimum* stratum shows where caching cannot help.

**Team and harness factors.**
- N ∈ {1, 3, 8}. N = 1 is the expected no-benefit control.
- **Harness R** is a reference worker built on a Maidan SDK and calling the Messages and Responses APIs directly, with explicit breakpoints. It gives full control.
- **Harness P** is one popular harness configured the way §1 describes (the Claude Agent SDK with `excludeDynamicSections`, or pi). It measures what real integrators get.
- Models: one Anthropic and one OpenAI model at fixed effort. Optionally a self-hosted vLLM model.

**Arms.**

| Arm | Coordination | Context | Caching |
|---|---|---|---|
| A0 | Maidan claims | Plain pack | **Disabled** (Anthropic: no `cache_control`; OpenAI 5.6+: explicit mode with no breakpoints, "does not use prompt caching") |
| A1 (honest baseline) | **None.** Agents pick from a shared list, so duplicates are possible | Each agent reads files itself | Provider best practice: automatic or implicit caching, stable system prompt, append-only history |
| A2 | Maidan claims | Plain packs, re-fetched in full | Provider best practice |
| A3 | Maidan claims | **Cache-stable**: fleet prefix, digests, deltas, stagger, TTL planner (§5) | Provider best practice |
| A3-perturbed | As A3 | Same bytes, but re-serialized per agent or with a timestamp at the head | As A3 |
| A4 (optional) | As A3 | As A3 | 1 h TTL with pre-warm (`max_tokens: 0`) |
| A5 (optional) | As A3 | As A3 | Self-hosted vLLM automatic prefix caching behind round-robin vs llm-d `prefix-cache-scorer` vs SGLang `cache_aware` |

How the comparisons isolate effects:
- **A2 − A1** isolates coordination.
- **A3 − A2** isolates cache-stable context.
- **A3 − A3-perturbed** separates byte stability from information content.
- **A0** bounds the total effect of caching.

**Isolation.**
- Each arm runs in its own Anthropic workspace (documented isolation; check the `anthropic-workspace-id` response header) and with its own OpenAI `prompt_cache_key`, which "separates cache reuse between groups of requests". On vLLM, each arm gets its own `cache_salt`.
- Within an arm, agents share a workspace, because cross-agent sharing is the effect being measured.
- Blocks are task-set × model × repetition, with arms interleaved in randomized order inside the same time windows.
- Log inter-call gaps.
- Request Anthropic `diagnostics` on every call and log `cache_miss_reason`. Log the response `model` id so silent model swaps are detectable.

**Metrics.**
- **Primary:** CPS = Σ billed $ over every attempt in the arm, divided by the number of successes.
- **Secondary:**
  - Token hit rate H = ΣR / Σ(U + R + W), where U is uncached input, R is cache reads and W is cache writes with 5-minute and 1-hour writes kept separate.
  - Write share W / (U + R + W).
  - Dollar-weighted input saving = 1 − (actual input $ / input $ priced at base rate).
  - Duplicate-attempt rate: tasks attempted by more than one agent.
  - Abandoned spend: spend on claims that never reached a result.
  - Turns per success, pass^k for k = 1..5, wall-clock time and TTFT.

**Statistics.**
- At least 5 repetitions per task per arm.
- Pilot on the dev split to estimate the ICC (prior 0.37–0.55), then size the holdout on *tasks*, not runs.
- Paired within-block differences, task-clustered bootstrap (10,000 resamples, fixed seed) for 95% confidence intervals, and Holm correction across arms.
- Report the ICC and the Kish effective n.

**Publish:**
- Every raw usage row, exported from Maidan's ledger (§4.5).
- A price-snapshot file: date, source URL, page sha256, and promotional end dates.
- Reconciliation against the provider billing export, with the residual (about 1% in arXiv 2607.12161).
- Harness and model versions as commit SHAs and model ids.
- Prompts, pack hashes, tasks, graders, transcripts, analysis code and seeds.
- **Total spend, including exploratory runs.**

### 4.4 What makes it credible rather than marketing

1. **Public pre-registration before any holdout spend.** A signed git tag of a manifest containing task hashes, graders, analysis code, the non-inferiority margin and stopping rules (or an OSF registration). arXiv 2607.12161 had to call its own plan "pre-specified" rather than "pre-registered" because it lacked this.
2. **A baseline that follows the providers' own guidance** (A1). Publish A1's hit rate. If it is about 95%, say so plainly.
3. **Publish nulls and losses.** Report the N = 1 stratum, below-minimum prefixes, long human pauses, and any arm where Maidan costs more. arXiv 2607.12161's +6.8% result is the standard of honesty to meet.
4. **Ablations that attribute the effect** (A2 vs A1, A3 vs A2, A3 vs A3-perturbed), not one headline percentage.
5. **Third-party reproduction.** One command reruns everything, and an independent operator is invited (HAL-style "third-party" standing). Report Pareto plots of success against CPS.
6. **Repriceability.** Publish tokens next to dollars. Recompute CPS under alternate price snapshots, batch rates and TTL choices. Rerun and re-date for each model generation.
7. **Disclosure of conflicts** and of every deviation from the registered plan.

### 4.5 What Maidan's ledger needs to support this

`report_usage` already records the four tiers `input`, `output`, `cache_read` and `cache_write`, with four snapshotted micro-USD rates. It verifies `usd_micros = ceil(sum(tokens × rate) / 1_000_000)` and is idempotent per `usage_report_id` (`docs/Integration.md` §4; `crates/maidan-types/src/usage_ledger.rs`, where `TokenUsage` uses `deny_unknown_fields`). That makes it a sound base. The gaps are:

1. **Split cache writes by TTL** (`cache_write_5m` and `cache_write_1h`, each with its own rate). Anthropic bills 1.25× and 2× within a single response (`cache_creation.ephemeral_5m/1h_input_tokens`), and pi already models `cacheWrite1h`. Today, a report that mixes both TTLs cannot be priced exactly.
2. **A normalization contract per provider.** Specify that `input` means *uncached* input (so OpenAI reporters must send `input_tokens − cached_tokens − cache_write_tokens`), and make the SDKs do the conversion.
3. **Optional evidence fields:**
   - provider;
   - response model id;
   - provider request id (to reconcile with billing);
   - `cache_miss_reason`;
   - a hash of the workspace or organization id;
   - harness name and version;
   - **the pack and snapshot hashes included in the call**, which lets Maidan compute hit rate per pack, the cross-agent metric no one else can produce.
4. **Ingest instead of self-report.** Accept OTel GenAI spans (`gen_ai.usage.cache_read.input_tokens` and `gen_ai.usage.cache_write.input_tokens` are emitted by Codex, Goose and the OpenHands SDK; Claude Code emits `claude_code.token.usage`) and reconcile them with `report_usage`.

---

## 5. Art of the possible: ten ideas for a coordinator in this position

Each idea gives what it is, why it is feasible, what it depends on, how to measure it, and the main risk. They are ordered roughly from most certain to most ambitious.

### Idea 1. A cache-stable surface as a contract (do first)

**What**
- Implement `server/discover` and send real `ttlMs` and `cacheScope` values:
  - long TTLs and `"private"` on capability-filtered lists;
  - the maximum TTL on `maidan://artifacts/{sha256}`;
  - `subscriptions/listen` invalidation.
- Add a CI contract that `tools/list`, `server/discover` and the `instructions` are byte-identical across replicas and across two tenants holding the same capability set.
- Rewrite the `instructions` for the worst placement (H3).
- Record harness `cache_miss_reason` values in the ledger and correlate them with Maidan events such as deploys, grant changes and `list_changed`.

**Why feasible**
- Mostly internal work. The spec requires it (§2), the SDKs already consume it, and a conformance suite checks it (`sep-2549-*`, `tools-list-deterministic-order`).
- Anthropic cache diagnostics are generally available.

**Depends on:** nothing external.

**Measure:** conformance pass. Zero Maidan-attributable `tools_changed` misses in harness diagnostics.

**Risk:** `cacheScope: "public"` misuse across tenants. Default to `"private"` and add a two-tenant test.

### Idea 2. Server-side tool profiles: a context diet on the most expensive layer

**What**
- A token or named capability set selects a tool *profile*. The default worker profile is the seven-tool hero loop (≈2k tokens, versus ≈27.6k tokens for the full catalog).
- The full catalog stays available to harnesses that defer tools.
- Name prefixes are consistent, so host-side search matches whole groups.

**Why feasible**
- `catalog_for(auth)` already filters by capability (`tools/mod.rs` L59–72).
- Six harness families resend every tool on every request (§1.3).
- SEP-2053's reference implementation (`modelcontextprotocol/experimental-ext-variants`) shows the idea of per-agent tool surfaces, though that SEP is closed pending the working group.
- Arithmetic for a 50-turn task on Sonnet 5.5: about $0.34 of cached tool definitions with the full catalog versus about $0.025 with the hero loop, per agent per task.

**Depends on:** fleets on a shared capability set, so tool bytes stay identical within a fleet.

**Measure:** tools-prefix tokens per request and CPS, A2 vs A3.

**Risk:** agents missing a tool they need. Mitigate with a "widen" tool or a fallback to deferral.

### Idea 3. Fleet prefix: a shared "room prefix" for homogeneous fleets

**What**
- Maidan publishes a byte-stable, content-addressed room preamble: glossary, accepted decisions, room rules, and a digest of the repo or corpus.
- Fleets that Maidan launches or configures put it immediately after an identical harness prefix:
  - in the static part of a custom system prompt before `SYSTEM_PROMPT_DYNAMIC_BOUNDARY`, in the TS Agent SDK with `excludeDynamicSections`;
  - or directly, in Maidan-SDK reference workers.
- All workers in a fleet then read one cache entry.

**Why feasible**
- The Agent SDK documents that identical configurations "share a cache entry across users and machines".
- Claude Code's workflow stagger proves the sibling-prefix pattern works.
- Arithmetic: a 50k-token pack used by 8 agents within the TTL on Sonnet 5.5 costs $1.00 as 8 separate writes, or $0.195 as 1 write plus 7 reads.
- Where exactly the room text lands inside the Agent SDK prompt is my inference from its docs (**UNVERIFIED** in practice).

**Depends on**
- Same API workspace (Anthropic) or organization (OpenAI); caches are isolated per workspace.
- Same harness build and configuration, same tool profile, and a prefix above the model minimum.
- Recency: a 5-minute TTL by default.

**Measure:** write share and CPS, A3 vs A2 and A3 vs A3-perturbed, at N = 1, 3 and 8.

**Risk:**
- Heterogeneous harnesses get nothing.
- A prefix that changes at every LSN never hits. Version the prefix in coarse epochs (e.g. hourly or at decision boundaries), as Goose does with hour-granularity time.

### Idea 4. A TTL, pre-warm and stagger planner

**What**
- Maidan knows queue depth, claim rate and the fleet's members, so it can forecast how many reads a prefix will get within its TTL. It then tells workers in the claim response:
  - which TTL to use (1 h only when it forecasts at least 2 reads; 5 minutes otherwise);
  - whether to pre-warm with `max_tokens: 0`;
  - when to start. Siblings are held until the first response begins, because "a cache entry only becomes available after the first response begins" (Anthropic).

**Why feasible**
- The break-even arithmetic: a 5-minute write wins on the first reuse (1.35 vs 2.0 units); a 1-hour write needs at least 2 reads (2.1 vs 2.0 with one).
- Pre-warming bills zero output tokens.
- pi already gates its own cache warming on expected savings of at least $0.05.
- Claude Code's stagger holds siblings for up to 5,000 ms.
- Claims are server-side, so Maidan can impose the stagger itself.

**Depends on:** workers honoring the hint (reference workers and pi-style settings can; most other harnesses cannot), and timestamps in the ledger.

**Measure:** arm A4 vs A3. Hit rate on first requests after a fan-out.

**Risk:** added latency from the stagger. Cap it, as Claude Code does.

### Idea 5. Context diets via deltas: send changes, not re-reads

**What**
- `get_thread_context` accepts `since=<pack sha or LSN>` and returns only what was appended (new messages, FSM transitions, new decisions), plus the new pack hash.
- Maidan records which member received which pack hash (implicit read receipts), so a re-claim or follow-up turn gets a delta by default.

**Why feasible**
- Maidan already has an append-only event log with LSNs, `as_of` replay, content-addressed snapshots and a `token_budget` with auditable elision (`docs/Integration.md`, "Fidelity & context").
- Appending keeps the agent's prefix intact, in line with Manus's "Make your context append-only".
- Rewriting the context is what raised billed cost by 6.8% in arXiv 2607.12161.
- MCP Agent Mail's `unread_only` shows demand for the idea, though only qualitatively.

**Depends on:** a store for per-member read receipts, and harnesses that keep earlier tool results (compaction breaks this; Cline caps results at 8,000 characters).

**Measure:** tokens read per claim and turns per success.

**Risk:** an agent misses context it never actually saw because compaction dropped it. Provide a "full pack" escape hatch.

### Idea 6. Summarize once, reuse many: content-addressed digests

**What**
- A digest of a channel or thread up to LSN X is produced once, by a cheap model as a Maidan task of its own, and stored as an immutable artifact.
- Other agents reference it by hash: as a `ResourceLink`, as a Skills-extension file with its `sha256` digest, or as part of the fleet prefix (Idea 3).

**Why feasible**
- Anthropic's own pattern returns "a condensed, distilled summary… (often 1,000-2,000 tokens)" and recommends passing "lightweight references back".
- The Skills extension (Final) makes digest-validated caching standard: "a cached file whose digest matches the current entry can be served without fetching it again".
- Maidan's artifact store already dedupes by sha256.

**Depends on:**
- A freshness policy: digests are pinned to an LSN and regenerated at epoch boundaries.
- Quality control, through Maidan's existing review flow for the digest task.
- Skills support in hosts is still partial (§2).

**Measure:** context tokens per claimant and CPS with and without digests (an A3 sub-arm).

**Risk:** errors propagate from one bad digest to many agents. Gate digests on review and keep a pointer to the source so agents can verify.

### Idea 7. De-duplicating identical work, and exact reuse of completed results

**What**
- At thread creation, seeding or claim time, Maidan computes a **task fingerprint**: a hash of the canonical task spec, the input-snapshot hashes, and the model class.
- If an identical fingerprint is already claimed, Maidan refuses or links the new claim.
- If it already has an *accepted* result, Maidan offers that result before any LLM spend.
- Near-matches are offered only as hints, never as answers.
- The ledger prices each avoided duplicate at the holder's actual cost.

**Why feasible**
- Claims are already exclusive and fenced per thread.
- Agentic Plan Caching reports "reduce costs by 50.31%" from reuse "from **completed** agent executions".
- Waste from duplicate work is documented by Anthropic ("agents duplicate work"), MAST (15.7% step repetition) and Claude Code agent teams ("overwrites").
- Exact hashing avoids the semantic-cache failure LiteLLM documents.

**Depends on:**
- Canonical task specifications. Free-text asks rarely collide exactly, so this works best for structured tasks such as "review PR at SHA", "summarize thread to LSN" or "run eval X".
- Review outcomes to mark a result reusable.

**Measure:** duplicate-attempt rate and avoided dollars, A2 vs A1.

**Risk:** a false equivalence when the inputs are not fully captured. Fingerprint only explicit snapshot hashes.

### Idea 8. A batch lane for tasks with slack deadlines

**What**
- Threads carry a deadline or slack hint (a new field; today threads have budgets with `max_wall_secs`, not due times).
- A batch worker claims slack-tolerant, few-turn tasks (digests, reviews, classification, eval items) and runs them through the provider's cheaper lanes:
  - Anthropic Message Batches;
  - OpenAI Batch or Flex;
  - DeepSeek off-peak hours.
- Usage is reported at the lane's rates.

**Why feasible**
- Anthropic: "All usage is charged at 50% of the standard API prices", and the caching multipliers "stack with other pricing modifiers such as the Batch API discount" (pricing page).
- OpenAI Batch is 50% off. Flex is "priced at Batch API rates, with additional discounts from prompt caching", and a 429 on Flex is not charged.
- DeepSeek: "Off-peak rates are half of the peak rates".
- The coordinator is the only layer that knows a task's deadline.

**Depends on:** the lane fitting the task. Multi-turn tool loops fit Flex better than 24-hour batches. Batch cache hits are best-effort (30–98%), and batch/cache stacking differs by model (Gemini 3.1 Pro's does not stack).

**Measure:** CPS by lane, and the share of the backlog that is eligible.

**Risk:** deadline misses. Promote a task back to the interactive lane when its slack runs out.

### Idea 9. Model and effort routing by task class, learned from the ledger

**What**
- Maidan computes CPS and success rate for each combination of task class, model and effort, using its own ledger joined to result and review outcomes.
- It recommends `suggested_model` and `suggested_effort` in the claim response.
- Routing happens once per task, never per turn.

**Why feasible**
- RouteLLM ("reduce costs by up to 85% while maintaining 95% GPT-4 performance") and TensorZero episodes show the payoff for outcome-labelled routing.
- Maidan already has every input except task-class labels.
- Routing per turn is self-defeating: in Claude Code a model switch "reads the entire conversation history with no cache hits", and `output_config.effort` invalidates the messages cache on Anthropic. Per-task routing preserves the cache.

**Depends on**
- Enough outcomes per class; review is the label source.
- Harnesses honoring the hint. Claude Code agent frontmatter `model` and the Agents SDK `ModelSettings` can; others vary.

**Measure:** CPS by class, before and after routing, with a non-inferiority check on success.

**Risk:** Goodhart effects on review labels. Keep humans or blind graders in the loop for the classes used to train routing.

### Idea 10. Cache-affinity claiming and KV-routing hints

**What**
- **(a) Hosted APIs: warm-cache claiming.** `claim_next_thread` prefers giving an agent the next task whose prefix it consumed within the current TTL. Maidan infers this from ledger timestamps and the pack hashes in the calls (§4.5).
- **(b) Self-hosted.** Maidan emits a stable `affinity_key`, a hash of workspace and prefix epoch. Workers forward it as:
  - llm-d `session-id-producer` header (e.g. `x-session-id`) or `x-session-token` for `session-affinity-filter`;
  - vLLM production-stack `--session-key`;
  - OpenAI `prompt_cache_key` before GPT-5.6;
  - xAI `x-grok-conv-id`;
  - A2A `contextId`.
- Maidan also maps thread priority and deadline to NVIDIA Dynamo `nvext.agent_hints.priority` / `osl`, and sets a per-tenant vLLM `cache_salt`.

**Why feasible**
- OpenRouter already does sticky routing for cache hits (10-minute expiry), and Maidan can do the same at *task* granularity.
- Self-hosted routers accept exactly these hints (llm-d inference scheduler v0.10.0, 2026-09-29; vllm-project/production-stack; ai-dynamo/dynamo v1.5.0 docs).
- SGLang's "radix tree is not synchronized across replicas", so affinity matters there.
- Dynamo states "Neither the presence of a session ID nor `agent_hints` enables sticky sessions", so the router must be configured to use them.

**Depends on:** worker cooperation, self-hosted deployments for (b), and tenancy salts. Timing side channels on shared caches are documented (arXiv 2502.07776).

**Measure:** arm A5 (self-hosted) and the warm-claim hit rate for (a).

**Risk:** affinity versus fairness and queue latency. Bound the preference with an age cap.

**How the ideas depend on each other.** Ideas 1 and 2 are prerequisites with little uncertainty. Ideas 3–6 need the ledger fields from §4.5 to be measurable. Ideas 7–10 are where the coordinator's unique data (claims, completion, who read what) turns into savings no gateway or memory layer can offer, and each should be added to MCEB-1 as its own arm or stratum, not shipped on faith.

---

## 6. UNVERIFIED register

- How the TS SDK's `versionNegotiation: 'auto'` reacts to Maidan's `-32601` on `server/discover` (probably a legacy fallback).
- The real tokenizer count of Maidan's tool catalog. The ≈27.6k and ≈2.0k figures are 3.5 characters-per-token estimates from the Rust source.
- Claude Code's `mcp_instructions_delta` placement. It is from binary strings at 2.1.267 and may be flag-gated or changed.
- Where instructions go in the Agent SDK beyond the CLI behaviour.
- Cursor: request shape, `instructions` handling, breakpoint placement, `list_changed` handling, and the current state of the 40-tool cap.
- Codex: the 512-character instruction guidance (in docs, not code), and MCP prompts support.
- OpenAI Agents SDK: absence of deferral for local MCP; whether the JS SDK auto-generates a `prompt_cache_key`.
- Whether Goose or OpenHands send `prompt_cache_key` (not found by grep). CrewAI result truncation and cost. LangChain cost reporting and `prompt_cache_key`. pi tool ordering and MCP prompts. Cline SDK rollout percentage.
- Whether the Anthropic MCP connector and OpenAI remote MCP speak 2026-07-28 or honor `ttlMs`. Whether any host honors SEP-2419 `cache_hint`. Which hosts use `Resource.size`.
- Terminal-Bench `uncached_input_tokens` semantics. Per-entry caching status of SWE-bench bash-only costs. The OpenHands Index cost unit.
- The date Anthropic switched to workspace-level cache isolation (reported as 2026-02-05 only in secondary sources).
- Vendor figures without a method: Portkey's homepage percentage, Zep's "90%", MemOS "72%", AgentID and Fast.io coordination percentages.
- The exact placement of a room prefix inside the Agent SDK's prompt (Idea 3) is an inference from docs, not tested.

## 7. Where the evidence is kept

- Fetched docs: the pages cited inline (Claude Code, Agent SDK, Codex, Cursor) and (Anthropic, OpenAI, Gemini, Vertex, DeepSeek, xAI, Mistral, Bedrock and Azure caching and pricing pages).
- Clones:
  - shallow clones at the cited commits: modelcontextprotocol, typescript-sdk, python-sdk, progressive-disclosure-wg, experimental-ext-variants, ext-skills, A2A, codex, goose, OpenHands, software-agent-sdk, langchain, langchain-mcp-adapters, langgraph-bigtool, crewAI, crewAI-tools, cline, cline-legacy, aider, pi-mono.
  - and saved copies of: A2A spec and proto v1.0.0/v1.0.1, provider MCP and tool-search docs, openai-python.
- Claude Code binary strings were extracted from version 2.1.267.
- Maidan: `main` @ `94836a1c`. Key files:
  - `crates/maidan-mcp/src/server.rs` L500–556 (dispatch, initialize, instructions)
  - `crates/maidan-mcp/src/tools/{catalog.rs,mod.rs}`
  - `crates/maidan-types/src/usage_ledger.rs`
  - `crates/maidan-server/src/thread_context.rs`
  - `docs/{Integration.md,Benchmark.md,Framework Integrations.md,Open Work.md}`
