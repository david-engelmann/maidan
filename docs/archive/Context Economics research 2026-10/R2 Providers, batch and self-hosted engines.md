# R2: Prompt and context caching, discounts, self-hosted KV reuse, and what a coordinator can do

> Research input to [Context Economics](../../Context%20Economics.md), dated 2026-10-01. Kept as evidence: every claim carries its source and access date. The fetched pages and clones it cites were working copies and are not committed; the URLs and `repo@commit` citations make each claim re-checkable.


Prepared for: the Maidan context-economics program
Retrieved: 2026-09-30 to 2026-10-01 (all sources, UTC 2026-10-01). Every source in the index (section 8) was fetched on those dates. Facts cite a source ID like `[A1]`.

**Method note.** The Exa search tools needed re-authentication, so I found sources with WebSearch and fetched them with `curl`. I used the Markdown versions of the provider docs where they exist (`.md` / `.md.txt`). Two parallel research sub-agents hit the account rate limit partway through. I finished their sections (3 and 4) myself, using the pages they had saved plus new fetches. Facts I could not confirm from a primary source are marked **UNVERIFIED**. Facts from sources older than about six months (before 2026-04) are marked **[>6mo]**.

**The docs have moved a lot since early 2026.** Current model names include Claude Opus 5.5 / Fable 5.1, GPT-5.6 / GPT-6.1 Sol and Gemini 3.8 Flash. OpenAI added explicit cache breakpoints and a cache-write premium for GPT-5.6+. Anthropic added `max_tokens: 0` pre-warming and cache diagnostics. Do not mix this report with memory from 2025.

---

## 0. Executive summary

1. **Every major hosted provider now caches exact prefixes, but scope, TTL, write premium and accounting fields all differ.**
   - Anthropic charges 1.25x (5 min) or 2x (1 h) to write and 0.1x to read. Reads are 0.05x on Opus 5.5 and 0.025x on Fable 5.1 / Mythos 5.1. Caches are isolated per *workspace* [A1][A2].
   - OpenAI GPT-5.6+ charges 1.25x to write, 0.1x to read (0.05x on GPT-6.1 Sol), with a 30-minute minimum TTL. Older models have no write fee. Caches are scoped per organization and processing region [O1][O2].
   - Gemini has implicit caching with no guarantee, plus explicit `cachedContents` objects that bill storage per token-hour [G2][G3].
   - DeepSeek cache hits cost 2% of a miss on `deepseek-flash`, and off-peak hours are half price [D2].
2. **The cache is per model and, at OpenAI, per service tier.** Switching model, service tier (flex/standard/fast), tool list, reasoning effort, output schema, or Anthropic's fast-mode `speed` setting breaks the prefix [A1][A3][O1][O3].
3. **Parallel cold starts are the single largest avoidable waste for teams of agents.** On Anthropic a cache entry exists only *after the first response begins* [A1].
   - Example (derived): N agents fanning out on a shared 40k-token pack on Sonnet 5.5 pay N cache writes.
   - With N = 8 that is $0.80 for the shared prefix. Warming once and then fanning out costs $0.156, and sending without caching costs $0.64. Parallel cold writes cost *more than not caching at all*.
   - NVIDIA measured this in Claude Code agent teams: teammates averaged 79.4% hit rate against 91.3% for explore subagents, "driven almost entirely by cold-start writes" [N3].
4. **In agentic coding, cache reads and writes already dominate the bill.**
   - Cache creation plus reads were about 87% of reconstructed cost (about 80% of the actual bill) [P-trnc].
   - The harness base (system prompt plus tool definitions) was 71.6% of context and 74.7% of cost [P-trnc].
   - Claude Code and Codex traces serve about 96% of prompt tokens from cache, yet still prefill 5.3x more than an ideal cache would. The misses cluster after human-paced gaps longer than 5 minutes [P-tracelab].
5. **Harness prefixes are fragile across agents.** Claude Code puts the working directory, platform, OS and a git-status snapshot into the conversation, and the auto-memory path into the system prompt. Agents in different directories or machines therefore miss each other's cache [A9][A10]. A coordinator can standardize these.
6. **Self-hosted stacks now accept exactly the hints a coordinator can provide:**
   - session and parent-session headers: Dynamo `X-Dynamo-Session-ID` / `X-Dynamo-Parent-Session-ID` / `X-Dynamo-Session-Final` [N2]; TensorRT-LLM `x-trtllm-subagent-affinity-id` [T3]; AIBrix `x-aibrix-session-key` [K2]; llm-d `x-session-id` [L3];
   - routing keys: SMG `X-SMG-Routing-Key` and `x-smg-routing-tokens` [S5][S6];
   - session lifecycle: SGLang `session_id` plus `/close_session` [S3];
   - retention priorities: SGLang `priority` eviction, TensorRT-LLM `KvCacheRetentionConfig`, Dynamo `nvext.agent_hints` [S2][T2][N1].

   llm-d's stated agentic direction is to build a session graph from hints such as Anthropic `cache_control` and OpenAI `prompt_cache_key` [L4].
7. **Non-prefix KV reuse is self-hosted only.** Research such as KVCOMM, CacheBlend, DroidSpeak and EPIC reports 70–87% reuse and 2–8x faster TTFT, but needs control of the engine. Hosted APIs reuse exact prefixes only.
8. **What only a coordinator can do:**
   - lay out shared context in a byte-stable, cache-ordered way;
   - schedule warm-then-fan-out;
   - keep prefix groups on one model, tier, account and routing key;
   - time follow-ups inside TTLs;
   - route deadline-tolerant work to batch, flex or off-peak lanes;
   - normalize cache accounting into one ledger.

   **The hard limit:** caches never cross provider organizations or workspaces, models, or (at OpenAI) regions. Maidan does not hold the API keys, so it can only shape the inputs and the schedule, plus optional gateway hints.

---

## 1. Hosted prompt/context caching, per provider

### 1.1 Anthropic: Claude API (first-party)

**Mechanism: explicit, prefix-based, with an "automatic" convenience mode** [A1].
- **Explicit mode.** Put `cache_control: {"type":"ephemeral"}` on content blocks, up to **4 breakpoints**.
- **Automatic mode.** A single *top-level* `cache_control` field moves the breakpoint to the last cacheable block as a conversation grows.
- **Prefix order.** The prefix is hierarchical: `tools` → `system` → `messages`.
- **Writes and reads.**
  - Writes happen *only at breakpoints*. Each write is "a hash of the prefix ending at that block".
  - Reads look backward up to **20 blocks** per breakpoint for entries that earlier requests wrote.
  - On the Claude API, a run of consecutive `tool_use` blocks counts as one position, and so does a run of `tool_result` blocks.
- **What invalidates the cache** [A1]:
  - any tool-definition change invalidates everything;
  - the web-search or citations toggle and the fast-mode `speed` setting invalidate system and messages;
  - `tool_choice`, adding or removing images, thinking parameters and `output_config.effort` invalidate messages (model-specific effects on system/tools).
- **Model scope.** "The cache is per-model" [A3].
- **Two newer mechanisms that avoid invalidation:**
  - mid-conversation `{"role":"system"}` messages on Opus 5.5, Fable 5.x, Opus 4.8, Opus 5 and Sonnet 5.5;
  - the beta header `inline-tools-2026-09-15`, which adds or changes tools via a `tool_addition` block without editing `tools` [A1].

**Key and scope** [A1].
- The cache key is a cryptographic hash of the prompt up to the cache-control point, and matching requires 100% identical segments.
- Isolation is **per workspace** on the Claude API, Claude Platform on AWS and Microsoft Foundry.
- Isolation is **per organization** on Bedrock and Google Cloud.
- Caches are never shared across organizations.

**TTL** [A1][A2].
- The default is **5 minutes**, refreshed at no extra cost on each hit. **1 hour** is optional (`"ttl":"1h"`) at 2x.
- The lifetime runs from the *start* of the request that writes or reads the entry, so generation time counts against it.
- When mixing TTLs, the longer-TTL entries must come first.
- There is no manual clear. Entries are "promptly, though not immediately" deleted after their minimum lifetime.

**Concurrency** [A1]. "A cache entry only becomes available after the first response begins. If you need cache hits for parallel requests, wait for the first response before sending subsequent requests."

**Pre-warming** [A1].
- `max_tokens: 0` writes the cache at the breakpoints and returns no output. Zero output tokens are billed, and the write is charged as normal.
- It is rejected with `stream: true`, with extended thinking, with structured outputs, with forced `tool_choice`, and inside Message Batches.
- Claude Code itself sends warm-up calls for its tool list and its Explore and Plan subagents before starting real work [P-lmcache-cc] **[>6mo, 2025-12-23]**.

**Minimum cacheable length** [A1]. Shorter prompts are silently not cached.

| Minimum | Models |
|---|---|
| 512 | Fable 5.1, Mythos 5.1, Opus 5.5, Opus 5, Sonnet 5.5, Fable 5, Mythos 5 |
| 1,024 | Opus 4.8, Sonnet 5, Sonnet 4.6, Sonnet 4.5, Opus 4.1, Opus 4, Sonnet 4 |
| 2,048 | Mythos Preview, Opus 4.7, Haiku 3.5 |
| 4,096 | Opus 4.6, Opus 4.5, Haiku 4.5 |

**Pricing** [A2], per MTok.

| Model | Input | 5m write | 1h write | Cache read | Output |
|---|---|---|---|---|---|
| Fable 5.1 | $10 | $12.50 | $20 | $0.25 (0.025x) | $50 |
| Opus 5.5 | $4 | $5 | $8 | $0.20 (0.05x) | $20 |
| Opus 5 / 4.8–4.5 | $5 | $6.25 | $10 | $0.50 | $25 |
| Sonnet 5.5 / 5 | $2 | $2.50 | $4 | $0.20 | $10 |
| Sonnet 4.6 / 4.5 | $3 | $3.75 | $6 | $0.30 | $15 |
| Haiku 4.5 | $1 | $1.25 | $2 | $0.10 | $5 |

- The multipliers stack with the Batch discount (50%) and data residency (`inference_geo:"us"`, 1.1x on 4.6+) [A2].
- Fast mode (research preview, first-party only) costs $8/$40 on Opus 5.5 and is not available with Batch [A2].
- The 1M context window is billed at standard rates on 4.6+, with caching and batch discounts applied across it [A2].

**Rate limits** [A5]. For most models only `input_tokens` plus `cache_creation_input_tokens` count toward ITPM, and **cache reads do not**. Haiku 3.5 is the exception.

**Diagnostics** [A3] (GA, Claude API only).
- Send `"diagnostics": {"previous_message_id": <prior id | null>}`.
- The response carries `diagnostics.cache_miss_reason.type`, one of `model_changed`, `system_changed`, `tools_changed`, `messages_changed`, `previous_message_not_found` or `unavailable`.
- The `*_changed` types also carry `cache_missed_input_tokens`.
- Fingerprints are scoped to org and workspace and expire after a short period.

**Usage fields** [A1]. All are in `usage`; for streaming they arrive in the `message_start` event.
- `input_tokens` counts **only the tokens after the last breakpoint** (not the total).
- `cache_creation_input_tokens` and `cache_read_input_tokens`.
- `cache_creation.ephemeral_5m_input_tokens` and `cache_creation.ephemeral_1h_input_tokens`, which sum to `cache_creation_input_tokens`.
- Total input = `cache_read_input_tokens + cache_creation_input_tokens + input_tokens`.
- `usage.service_tier` reports the tier used [A6].

**Priority Tier.** "Capacity commitments are no longer available for purchase." Existing contracts continue [A6].

### 1.2 Anthropic models on other platforms

- **Claude Platform on AWS** (Anthropic-operated, billed via AWS Marketplace in CCUs) [A2][A8].
  - Caching works as on the Claude API: workspace isolation, 5m and 1h TTLs, automatic caching.
  - Workspaces are bound to a single AWS region.
  - Cache diagnostics are *not* available here [A3].
- **Amazon Bedrock (AWS-operated)** [B1][A7].
  - Bedrock offers both *implicit* caching (best-effort, no controls) and *explicit* caching.
  - Explicit caching uses `cachePoint` in Converse or `cache_control` in InvokeModel, with up to 4 checkpoints for Claude.
  - "Simplified cache management" looks back about 20 blocks.
  - TTL is 5 min, or 1 h on listed models (`"ttl":"1h"` in `cachePoint` or `cache_control`).
  - Minimum per checkpoint is cumulative over the prefix:

    | Minimum | Models |
    |---|---|
    | 512 | Opus 5.5 / 5, Sonnet 5.5, Fable 5.x |
    | 1,024 | Opus 4.8, Sonnet 5 / 4.6 / 4.5 / 3.7 |
    | 4,096 | Opus 4.7 / 4.6 / 4.5, Haiku 4.5 |

    **Conflict:** Anthropic lists Opus 4.7 at 2,048 [A1], Bedrock at 4,096 [B1].
  - "Prompt caching is only supported for on-demand inference endpoints. It is **not supported with the batch inference API**" [B1].
  - Cross-region inference "may lead to increased cache writes" [B1].
  - Automatic (top-level) caching is not available on the legacy Bedrock integration for Opus 4.6 and earlier, which returns a 400 [A1].
  - Scope is organization-level per Anthropic [A1]. AWS does not state an account- or region-level scope (**UNVERIFIED**).
  - Usage (Converse) [B1]: `usage.inputTokens` (non-cached only), `cacheReadInputTokens`, `cacheWriteInputTokens` and `cacheDetails` (TTL per write). Total = `inputTokens + cacheReadInputTokens + cacheWriteInputTokens`.
  - Usage via InvokeModel or the new `/anthropic/v1/messages` Bedrock endpoint is presumably Anthropic-native (**UNVERIFIED**). [A1] defers to the Bedrock docs for "usage-field names".
  - Regional endpoints cost 10% more than global [A7].
- **Google Cloud / Vertex ("Gemini Enterprise Agent Platform")** [V1].
  - "Caches are unique to your Google Cloud project and cannot be used by other projects." Anthropic calls this organization-level isolation [A1]; the two statements are likely the same thing, but **UNVERIFIED**.
  - TTL is 5 min, or 1 h (not on Claude 3.7 / 3.5 Sonnet or 3 Opus).
  - The Vertex page states reads are "90% cheaper", which does not reflect Anthropic's 0.05x / 0.025x exceptions. The page is possibly stale, or Google prices differ (**conflict**).
  - Cache hashes are treated as "Service Data", not "Customer Data", and caching can be disabled per project via support.
  - Regional and multi-region endpoints cost 10% more [A2].
- **Microsoft Foundry.** Workspace isolation, 5m and 1h TTLs [A1].

### 1.3 OpenAI (direct API)

**Mechanism.** Caching is automatic and prefix-based. On **GPT-5.6 and later** it also supports explicit breakpoints [O1][O4].

- **Implicit mode (default).** A breakpoint goes at the end of the latest eligible message. Eligible messages are user messages, the last tool response in a group, and the last developer message in the initial group [O1].
- **Explicit mode.**
  - Set `prompt_cache_options.mode:"explicit"` and mark content blocks with `prompt_cache_breakpoint: {"mode":"explicit"}`. Supported blocks are `input_text`, `input_image` and `input_file`; top-level `instructions` cannot carry one [O1][O4].
  - Up to **4 cache writes per request**. Implicit mode spends one slot on its own breakpoint [O4].
  - Content after the last selected breakpoint is billed as uncached input with no write charge [O1].
- **Lookup boundaries.**
  - The guide says: "first 2 and latest 50 explicit breakpoints", plus the implicit breakpoint, up to 20 earlier eligible message endings, and the end of the initial developer block [O1].
  - The API reference says: "up to the latest 80 breakpoints in the conversation, without a content-block lookback limit" [O4].
  - These conflict (**conflict**).
- **Earlier models.** Implicit only, with breakpoints at model-dependent intervals (2,048 tokens for GPT-5.5). `cached_tokens` is rounded down to a multiple of 128 and excludes hidden system tokens [O1].
- **Settings that change the prefix** [O1]: `model`, `tools`, `parallel_tool_calls`, `text.format`, `reasoning.effort`, `text.verbosity`, `context_management` (compaction).
- **Diagnostics also treat a `service_tier` change and a `prompt_cache_key` change as misses** [O3]. So flex, standard and fast traffic do not share a prefix.

**Cache location, routing and scope** [O1].
- Cached states "live on individual machines". Traffic above **about 15 requests per minute** per prefix and key "can lead to overflow routing".
- Routing depends on machine load, a hash of the initial tokens (after hidden content, including tool definitions), and `prompt_cache_key`.
- Caches are not shared across organizations and "cannot be reused across regional processing boundaries" (data residency).
- On models before 5.6, use a stable `prompt_cache_key` per shared-prefix group and partition busy groups deterministically.
- On 5.6+, routing is automatic. The key is used for separate cache accounting and to prevent cache-hit probing across customers.
- `prompt_cache_key` replaces the old `user` field for caching [O4].
- Whether caches are shared across *projects* within an organization is not stated (**UNVERIFIED**; routing is described "within an organization and processing region").

**TTL** [O1][O4].
- GPT-5.6+: `prompt_cache_options.ttl` accepts only `"30m"`, which is also the default. Entries live at least 30 min after the last write or reuse, and may be retained longer.
- Earlier models: the deprecated `prompt_cache_retention` field.
  - `in_memory` is typically 5–10 min inactive, up to 1 h.
  - `24h` is typically about 30 min, up to 24 h.
  - The default is `24h` for non-ZDR organizations on supporting models. `gpt-5.5` supports only `24h`.

**Prewarm** [O1]. `prompt_cache_options.prewarm: true` prepares the cache without output and is billed at the cache-write rate.

**Minimum length** [O1]. 1,024 visible tokens on 5.6+. On earlier models it "varies by request settings".
- OpenAI gives a break-even formula for padding a short prefix up to the minimum: `L_break-even = M(r + (w−r)/N)`.
- Example: with M=1,024, r=0.1, w=1.25 and 10 requests, padding pays off for original prefixes of at least 221 tokens.

**Pricing** [O1][O2]. GPT-5.6+ writes cost 1.25x and reads 0.1x (0.05x on GPT-6.1 Sol). Write pricing "is not an additive fee". Selected short-context (≤272K) prices per MTok:

| Model | Input | Cached | Cache write | Output |
|---|---|---|---|---|
| gpt-6.1-sol | $2.00 | $0.10 | $2.50 | $10.00 |
| gpt-6-astra | $10.00 | $1.00 | $12.50 | $50.00 |
| gpt-5.6-sol (promo through at least 2026-11-21) | $4.00 | $0.40 | $5.00 | $20.00 |
| gpt-5.6-luna | $0.20 | $0.02 | $0.25 | $1.20 |
| gpt-5.5 | $5.00 | $0.50 | none | $30.00 |
| gpt-5.4 | $2.50 | $0.25 | none | $15.00 |
| gpt-4.1 | $2.00 | $0.50 | none | $8.00 |

Long context (>272K) doubles input [O2]. Regional processing costs +10% for models released on or after 2026-03-05 [O2].

**Rate limits.** "Cached input tokens still count toward tokens-per-minute limits" [O1]. This is the opposite of Anthropic.

**Diagnostics** [O3] (Responses API, GPT-5.6+).
- Send `prompt_cache_options.comparison_response_id`.
- The response carries `prompt_cache_diagnostics.type`, one of `cache_hit`, `cache_miss` (with `reason`, `cache_missed_tokens` and optional `comparison_reusable_tokens`), `comparison_response_not_found` or `unavailable`.
- Reasons are `model_changed`, `prompt_cache_key_changed`, `service_tier_changed`, `tools_changed`, `text_format_changed`, `reasoning_effort_changed`, `verbosity_changed`, `context_compacted` and `input_changed`.

**Usage fields.**
- Responses [O1][O4]: `usage.input_tokens` (the **total**, including cached and written tokens), `usage.input_tokens_details.cached_tokens` and `usage.input_tokens_details.cache_write_tokens`. Ordinary input = `input_tokens − cached_tokens − cache_write_tokens`.
- Chat Completions [O5]: `usage.prompt_tokens` (total), `usage.prompt_tokens_details.cached_tokens` and `usage.prompt_tokens_details.cache_write_tokens` ("the unadjusted number of prompt tokens written to cache"). There are also `audio_tokens`, `image_tokens` and `text_tokens`.

### 1.4 Azure OpenAI (Microsoft Foundry) [Z1]

- The API structures match OpenAI. You set `model` to your deployment name.
- **GPT-5.6+ `prompt_cache_key` differs from OpenAI.** Azure says to set it to "improve cache matching", with about 15 RPM per prefix and key before misses. OpenAI says the key is "not needed to optimize caching" on 5.6+ [O1] (**conflict / platform difference**).
- Explicit breakpoints and `cache_write_tokens` exist on **Standard pay-as-you-go only, not on PTU-M**.
- `prompt_cache_options.ttl` accepts only `30m`.
- Earlier models:
  - `in_memory` retention clears within 5–10 min of inactivity and always within 1 h;
  - `24h` extended retention "offload[s] the key/value tensors to GPU-local storage";
  - the default is `in_memory` for gpt-5.4 and older, and extended for gpt-5.5.
- **Scope:** "The system doesn't share prompt caches between Azure subscriptions."
- **Minimum:** 1,024 tokens, and the first 1,024 tokens must be identical. Before 5.6, hits come in 128-token increments.
- **Pricing:** cached reads are discounted on Standard and "up to 100% discount on input tokens for Provisioned". Writes may be charged on 5.6+.
- **Data residency:** Data Zone and Regional deployments keep extended-cache data inside the boundary.
- **Usage:** `prompt_tokens_details.cached_tokens` and `cache_write_tokens`.

### 1.5 Google Gemini API (AI Studio / Developer API)

**Implicit caching** [G1][G2].
- Enabled by default on Gemini 2.5 and newer, with "no cost saving guarantee".
- Minimum: 4,096 tokens for Gemini 3.8 / 3.7 / 3.6 / 3.5 Flash and 3.1 Pro Preview; 2,048 for Gemini 2.5 Flash and Pro.
- Google's advice: put large common content first and send similar prefixes close together in time.
- The **Interactions API supports implicit caching only** [G1].
- Implicit TTL and scope (API key vs project) are not documented (**UNVERIFIED**).

**Explicit caching** [G2]. Beta, `v1beta`, generateContent API.
- Create a `cachedContents/{id}` object bound to a model, then pass `cached_content` or `cachedContent` in requests. The OpenAI-compatibility layer takes `extra_body.cached_content`.
- TTL defaults to **1 hour**, with "no minimum or maximum bounds". Only `ttl` or `expire_time` can be updated.
- The content cannot be read back; only metadata can (`name`, `model`, `display_name`, `usage_metadata`, `create_time`, `update_time`, `expire_time`).
- "Token limits include cached tokens", meaning rate limits are not relieved by caching [G2].

**Pricing** [G3], per MTok, Standard tier.

| Model | Input | Cached | Storage |
|---|---|---|---|
| Gemini 3.8 Flash, through 2026-12-31 | $0.75 | $0.075 (0.1x) | $0.50 per MTok per hour |
| Gemini 3.8 Flash, from 2027-01-01 | $1.50 | $0.15 | $1.00 per MTok per hour |
| Gemini 3.1 Pro Preview (≤200k) | $2.00 | $0.20 | $4.50 per MTok per hour |
| Gemini 2.5 Pro (≤200k) | $1.25 | $0.125 | $4.50 per MTok per hour |

- There is no write premium. Explicit cache creation is billed at the standard input price, and implicit caching has no storage fee (per Vertex [V2]).

**Usage fields** [G4][G1].
- REST `usageMetadata.promptTokenCount` is the total effective prompt and *includes* cached tokens when `cachedContent` is set.
- `usageMetadata.cachedContentTokenCount`, `usageMetadata.cacheTokensDetails[]` (per modality) and `usageMetadata.serviceTier`.
- The Python and JS SDKs use snake_case (`usage_metadata.cached_content_token_count`).
- The Interactions API uses `usage.total_cached_tokens` [G1].
- Storage charges do not appear on generate responses. They must be computed from cache metadata (token count × TTL hours).

### 1.6 Vertex AI Gemini ("Gemini Enterprise Agent Platform") [V2][V3]

- **Implicit caching** is on for all Google Cloud projects at a 90% discount, and also covers "open models" served as a service.
- **Explicit caching** discounts 90% on 2.5+ and 75% on 2.0.
  - The default TTL is 60 min, with a minimum of 1 minute and no maximum.
  - Content is limited to 10 MB as a blob or text.
  - Caches are created as regional resources (`projects/{p}/locations/{l}/cachedContents/{id}`) and stored "in the region where you make the request".
- **Minimums:**
  - Gemini 3 family: 4,096.
  - "Gemini 3.0 Flash Preview, 3.1 Pro Preview, 3.7 Flash, and 3.8 Flash (implicit caching only): **6,144**" (**conflict** with the Gemini API's 4,096 [G2]).
  - Gemini 2 family: 2,048.
- **Provisioned Throughput:** "Caches work across traffic types." A cache created on Provisioned Throughput also works with PayGo.
- **Interaction between modes:** explicit caches can cause extra implicit caching ("disable implicit caching and avoid creating explicit caches" to prevent data retention).
- **Usage field:** `cachedContentTokenCount`.

### 1.7 Amazon Bedrock for non-Anthropic models [B1][B2]

- **Amazon Nova:** implicit caching for all text prompts. Some Nova models also accept explicit `cachePoint`s.
- **OpenAI GPT-5.6 Sol / Terra / Luna on Bedrock (Responses API):**
  - both implicit and explicit modes (`prompt_cache_breakpoint`, `prompt_cache_options.mode`);
  - 1,024 minimum;
  - **30-minute minimum TTL**;
  - writes at 1.25x, reads at a 90% discount;
  - **cached tokens do not count toward the ITPM quota**, which differs from OpenAI-direct [O1].
  - Usage: `usage.input_tokens_details.cached_tokens` and `cache_write_tokens`.
- **GPT-5.5 / 5.4 on Bedrock:** implicit only, 1,024 minimum, no write fee.
- **Kimi 3 on Bedrock:** $3.00 input, $0.30 cache read, $3.75 cache write (30 min), $15 output (global CRIS) [B2].
- **Tiers:** Bedrock lists Standard, **Flex (50% discount)**, **Priority (75% premium)** and Reserved. Batch inference is "50% lower" for select models [B2].

### 1.8 DeepSeek [D1][D2][D3]

**Mechanism: automatic "Context Caching on Disk"**, enabled for everyone [D1].
- Because of sliding-window attention, cached prefixes are discrete **prefix units**, and a request hits only if it fully matches a unit.
- Units are persisted in three ways:
  - at the end of the user input and at the end of the model output of each request;
  - when the system detects a common prefix across requests;
  - at fixed token intervals for long inputs or outputs.
- Example: requests `A+B` then `A+C` both miss. The system then persists `A`, so a third request `A+D` hits.
- Building a unit takes seconds. Unused caches are cleared "usually within a few hours to a few days". The service is best-effort.

**Scope** [D3].
- Concurrency limits are counted per account.
- The `user_id` parameter isolates **KVCache, scheduling and content safety** per end user. It goes in the body for the OpenAI format, or `metadata.user_id` for the Anthropic format.
- **Agents meant to share a cache must not send different `user_id`s.**

**Minimum length.** Not stated in the current docs (**UNVERIFIED**).

**Pricing** [D2], per MTok.

| Model | Cache hit (off-peak / peak) | Cache miss (off-peak / peak) | Output (off-peak / peak) |
|---|---|---|---|
| `deepseek-flash` (V4.1-Flash) | $0.003 / $0.006 | $0.15 / $0.30 | $0.6 / $1.2 |
| `deepseek-v4-pro` | $0.022 / $0.044 | $0.66 / $1.32 | $1.98 / $3.96 |

- **Off-peak is half the peak rate.** Peak is 01:00–04:00 and 06:00–10:00 UTC, Monday to Friday, excluding Chinese public holidays. This is a time-shift discount a scheduler can exploit.
- Concurrency limits are 2,500 for flash and 500 for pro.

**Usage fields.** `usage.prompt_cache_hit_tokens` and `usage.prompt_cache_miss_tokens` [D1]. That they sum to `prompt_tokens` is inferred (**UNVERIFIED**).

### 1.9 Mistral [M1][M2]

- **Mechanism:** automatic prefix caching. `prompt_cache_key` "increases the chance of a cache hit". Use a stable conversation, session or workflow ID.
- **Granularity:** 64-token cache blocks, so `cached_tokens` is a multiple of 64 and prompts under 64 tokens never hit.
- **Pricing:** cached tokens bill at 10% of input. No write fee is stated.
- **Usage:** `usage.prompt_tokens` counts all prompt tokens, and `usage.prompt_tokens_details.cached_tokens` is 0 or omitted on a miss. Billable uncached = `prompt_tokens − cached_tokens`.
- **TTL and scope:** not documented (**UNVERIFIED**).
- **Batch:** a 50% discount, up to 1M requests per batch [M2].

### 1.10 xAI (Grok) [X1][X2]

- **Mechanism:** automatic prefix caching on message boundaries, with entries "stored per-server".
- **Affinity:**
  - Set the `x-grok-conv-id` HTTP header on Chat Completions (or as gRPC metadata), or `prompt_cache_key` on the Responses API, to route a conversation to the same server.
  - A different conversation ID, or none, may land on a server without the cache.
  - Entries "can be evicted at any time due to server load or restarts".
- **Pricing** [X2], per MTok:

  | Model | Input | Cached | Output |
  |---|---|---|---|
  | grok-4.7 (<200k) | $2.00 | $0.50 (0.25x) | $6.00 |
  | grok-4.5 | $2.00 | $0.30 (0.15x) | $6.00 |
  | grok-4.3 | $1.25 | $0.20 (0.16x) | $2.50 |

  - No write fee is stated.
  - US regional endpoints cost 1.1x on all token types, applied after cache discounts.
- **Usage:** Chat uses `usage.prompt_tokens_details.cached_tokens`; Responses uses `usage.input_tokens_details.cached_tokens`. `prompt_tokens` and `input_tokens` include cached tokens.
- **Minimum length and scope (team or account):** **UNVERIFIED**.

### 1.11 Others (via OpenRouter's docs; secondary for the underlying providers) [R1]

- **Moonshot:** reads at 0.25x, no write cost.
- **Groq:** reads at 0.5x, Kimi K2 only.
- **Alibaba Qwen:** explicit `cache_control` (Anthropic syntax), writes at 1.25x, reads at 0.1x, 5-min TTL.
- **OpenRouter itself:**
  - "provider sticky routing" keyed per account, per model and per conversation;
  - a conversation is identified by hashing the first system or developer message plus the first non-system message, or by an explicit `session_id` body field or `x-session-id` header (≤256 chars);
  - sticky sessions expire after 10 min idle.
  - Usage: `prompt_tokens_details.cached_tokens` and `prompt_tokens_details.cache_write_tokens`, plus a `cache_discount` field.
- **Conflict:** OpenRouter's constants list GOOGLE_CACHE_READ_MULTIPLIER 0.25, but current Gemini pricing is 0.1x [G3]. Treat OpenRouter's multipliers as stale.
- A secondary blog claims OpenRouter reported an 82.8% platform-wide hit rate (**UNVERIFIED**).

### 1.12 Comparison at a glance

| Provider | Mode | Scope (sharing boundary) | TTL | Min tokens | Write premium | Read price | Cached counts toward rate limit? |
|---|---|---|---|---|---|---|---|
| Anthropic API | explicit + "automatic" top-level | workspace | 5m (free refresh) / 1h | 512–4,096 by model | 1.25x / 2x | 0.1x (0.05x Opus 5.5, 0.025x Fable 5.1) | No (most models) |
| Claude on Bedrock | implicit + explicit | org (per Anthropic) | 5m / 1h | 512–4,096 | 1.25x / 2x | 0.1x | **UNVERIFIED** |
| Claude on Vertex | explicit | GCP project | 5m / 1h | as Anthropic | 1.25x / 2x | "90% cheaper" | **UNVERIFIED** |
| OpenAI GPT-5.6+ | implicit + explicit | org + region (+ key partition) | ≥30m | 1,024 | 1.25x | 0.1x (0.05x GPT-6.1 Sol) | Yes |
| OpenAI ≤5.5 | implicit | org + region | in_memory 5–10m / 24h | varies (≈1,024) | none | 0.1–0.5x by model | Yes |
| Azure OpenAI | as OpenAI | subscription | as OpenAI | 1,024 | 5.6+: yes | discounted; PTU up to free | **UNVERIFIED** |
| Gemini API | implicit + explicit objects | project (explicit); implicit **UNVERIFIED** | explicit: default 1h, any | 2,048 / 4,096 | none (storage $/MTok-h) | 0.1x | Yes (limits include cached) |
| Vertex Gemini | implicit + explicit | project + region | explicit: ≥1 min, no max | 2,048 / 4,096 / 6,144 | none (storage) | 0.1x (0.25x on 2.0 explicit) | **UNVERIFIED** |
| DeepSeek | automatic disk | account, partitioned by `user_id` | hours–days | **UNVERIFIED** | none | 2% (flash) / 3.3% (pro) of miss | n/a (concurrency limits) |
| Mistral | automatic + key | **UNVERIFIED** | **UNVERIFIED** | 64 | none stated | 0.1x | **UNVERIFIED** |
| xAI | automatic + conv-id | per server; account **UNVERIFIED** | evictable any time | **UNVERIFIED** | none stated | 0.15–0.25x | **UNVERIFIED** |

### 1.13 Usage-field normalization (for one ledger)

The key trap is that providers disagree on what "input tokens" means.
- **Anthropic and Bedrock Converse** report `input_tokens` / `inputTokens` *excluding* cache reads and writes.
- **OpenAI, Azure, Mistral, xAI, Gemini and OpenRouter** report a *total* that includes cached tokens.

| Source | total_input | cache_read | cache_write (TTL split) | uncached_input |
|---|---|---|---|---|
| Anthropic Messages [A1] | `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` | `cache_read_input_tokens` | `cache_creation_input_tokens`, split by `cache_creation.ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens` | `input_tokens` |
| Bedrock Converse [B1] | `inputTokens + cacheReadInputTokens + cacheWriteInputTokens` | `cacheReadInputTokens` | `cacheWriteInputTokens` (TTL in `cacheDetails`) | `inputTokens` |
| OpenAI Responses / Bedrock Responses / Azure [O1][B1][Z1] | `input_tokens` | `input_tokens_details.cached_tokens` | `input_tokens_details.cache_write_tokens` (5.6+ only; TTL implied 30m) | total − read − write |
| OpenAI Chat / Azure Chat [O5][Z1] | `prompt_tokens` | `prompt_tokens_details.cached_tokens` | `prompt_tokens_details.cache_write_tokens` | total − read − write |
| Gemini generateContent [G4] | `usageMetadata.promptTokenCount` | `usageMetadata.cachedContentTokenCount` | none per request; explicit-cache creation is billed separately as input, and storage as token-hours | total − read |
| Gemini Interactions [G1] | **UNVERIFIED** | `usage.total_cached_tokens` | none | **UNVERIFIED** |
| DeepSeek [D1] | hit + miss (inferred) | `prompt_cache_hit_tokens` | none | `prompt_cache_miss_tokens` |
| Mistral [M1] | `prompt_tokens` | `prompt_tokens_details.cached_tokens` (×64, may be omitted) | none | total − read |
| xAI [X1] | `prompt_tokens` / `input_tokens` | `prompt_tokens_details.cached_tokens` / `input_tokens_details.cached_tokens` | none | total − read |
| OpenRouter [R1] | `prompt_tokens` | `prompt_tokens_details.cached_tokens` | `prompt_tokens_details.cache_write_tokens` (+ `cache_discount`) | total − read − write |
| vLLM OpenAI server [V-vllm] | `prompt_tokens` | `prompt_tokens_details.cached_tokens` (only with prompt-tokens-details enabled) | none | total − read |

Also record, per call:
- `service_tier`: Anthropic `usage.service_tier`, OpenAI response `service_tier`, Gemini `usageMetadata.serviceTier`;
- model ID, region or endpoint type, batch flag, cache key or session ID used, and provider scope ID (Anthropic `anthropic-workspace-id` response header [A3]).

Section 5 proposes a canonical schema.

---

## 2. Batch, flex, priority and how they combine with caching

| Provider | Batch | Flex / sheddable | Priority / fast | Combines with caching? |
|---|---|---|---|---|
| Anthropic | Message Batches: 50% off input and output; ≤100k requests or 256 MB; most finish <1 h, expire at 24 h; results kept 29 days [A4] | none | Priority Tier no longer sold [A6]; Fast mode premium, first-party only, not with Batch [A2] | **Yes, multipliers stack** [A2]. Cache hits in batches are "best-effort" because requests run concurrently in any order. Recommended recipe: send one request with the shared prefix and a **1h** cache block first, then submit the rest [A1]. `max_tokens:0` not allowed in batches [A4]. |
| OpenAI | Batch: 50% off, 24 h window, separate rate-limit pool, ≤50,000 requests / 200 MB [O6] | `service_tier:"flex"` at Batch-rate prices "with additional discounts from prompt caching"; may return 429 (not charged); beta [O7] | Fast mode (Priority renamed 2026-07-30): `service_tier:"fast"`, up to 2.5x faster, 2x price on GPT-5.6 Sol; cached discounts apply; ramp limits [O8] | Pricing table lists cached-input and cache-write rates under Batch and Flex for GPT-5.x/6.x (e.g. gpt-6.1-sol batch $1.00 / $0.05 / $1.25) [O2]. For gpt-4.1, gpt-4o and o-series the Batch cached column is "-", which reads as no cache discount (inference). **A `service_tier` change is a cache miss** [O3], so flex and standard are separate cache populations. |
| Gemini API | 50% off; target 24 h; explicit `cached_content` usable per batched request [G5] | Flex: `service_tier:"flex"`, 50% off, synchronous, 1–15 min target, sheddable (503), no server fallback (preview) [G6] | Priority: +75–100% [G7] | Yes. Pricing table shows cache price per tier: batch and flex cached $0.0375 on 3.8 Flash, priority $0.135 [G3]. **Conflict:** batch doc says a hit pays "the standard context caching rates" [G5]. |
| Bedrock | Batch 50% lower, select models [B2] | Flex tier 50% discount [B2] | Priority +75% [B2] | **Batch does not support prompt caching** [B1]. Flex and Priority with caching: **UNVERIFIED**. |
| Azure OpenAI | Global Batch 50% below global standard, 24 h target, jobs not expired [Z2] | **UNVERIFIED** | **UNVERIFIED** | Cached-token reporting appears in batch outputs [Z2]; discount combination **UNVERIFIED**. |
| DeepSeek | none found | none | none | Off-peak (50%) applies to hit and miss prices alike [D2]. |
| Mistral | 50% off, ≤1M requests [M2] | none found | none found | **UNVERIFIED** |
| xAI | 20% off on grok-4.3 and grok-4.20 variants only; applies to cached tokens; batch requests don't count toward rate limits [X2] | none | 2x, applied after cache discounts; not with Batch [X2] | Yes (stated) |

Effective floor prices for a cache-read token (derived from the tables above):
- Anthropic Sonnet 5.5 batch read: 0.5 × $0.20 = **$0.10/MTok**.
- OpenAI gpt-6.1-sol batch or flex cached: **$0.05/MTok**.
- Gemini 3.8 Flash batch cached: **$0.0375/MTok**, plus storage if explicit.
- DeepSeek flash off-peak hit: **$0.003/MTok**.

---

## 3. Self-hosted inference: prefix caching, KV sharing, cache-aware routing

### 3.1 Engines

**vLLM: Automatic Prefix Caching (APC)** [V-vllm].
- **On by default** (`enable_prefix_caching: bool = True`).
- **Block hashing.** Each KV block is hashed from three things:
  - the parent block's hash;
  - the block's tokens;
  - "extra hashes": LoRA IDs, multimodal input hashes and `cache_salt`.
- **Hash algorithm.** `--prefix-caching-hash-algo` selects one of:
  - `sha256`, the default since v0.11, pickle-based and not reproducible across Python or vLLM versions;
  - `sha256_cbor`, reproducible and cross-language;
  - `xxhash` or `xxhash_cbor`, faster and non-cryptographic.
- **Match granularity.** `prefix_match_unit` (the hash block size) can be finer than the physical block.
- **Eviction.** LRU over a free queue of reference-count-zero blocks.
- **Isolation.** A per-request `cache_salt` is injected into the first block's hash, so only requests with the same salt share. It is meant for trust groups.
- **KV events** (`enable_kv_cache_events`, ZMQ publisher, default endpoint `tcp://*:5557`, optional replay endpoint):
  - `BlockStored` carries `block_hashes`, `parent_block_hash`, `token_ids`, `lora_name` and `medium`;
  - `BlockRemoved` carries `block_hashes` and `medium`;
  - `AllBlocksCleared`.
- **Metrics.** `vllm:prefix_cache_queries` and `vllm:prefix_cache_hits`.
- **Request fields.** `priority` (lower = earlier) and `kv_transfer_params`. Cached-token usage appears in `prompt_tokens_details.cached_tokens` only when prompt-tokens-details is enabled.
- **Source:** the design doc and code on `main`.

**SGLang: RadixAttention plus HiCache.**
- **Radix-tree prefix cache** [S1]. The paper reports up to 6.4x throughput **[>6mo, 2023/24]**.
- **Eviction policies** (`--radix-eviction-policy`) [S2]:
  - `lru` (default), `lfu`, `slru`;
  - `priority`, which evicts the lowest-priority request's prefix first;
  - `tlru`, "TEL-safe" tail eviction for agentic and multi-turn TTFT.
- **Scheduling policies** (`--schedule-policy`): cache-aware `lpm` (longest prefix match) and `dfs-weight`; cache-agnostic `fcfs` (the current default), `lof`, `random`, `priority` and `routing-key` [S4].
  - In-batch prefix dedup runs once a prefix exceeds 32 tokens (`IN_BATCH_PREFIX_CACHING_CHECK_THRESHOLD`).
- **Priority scheduling** via `--enable-priority-scheduling` [S4].
- **Session-aware radix cache** (`--enable-session-radix-cache`) [S3].
  - The client passes the same top-level **`session_id`** on every request, and calls **`/close_session`** when done.
  - Session-referenced KV is evicted after unreferenced KV. This is soft protection, not pinning.
- **HiCache** extends the radix tree to host memory and storage, with backends `file`, `mooncake`, `hf3fs`, `nixl` and `aibrix`. LMSYS reports up to 6x throughput and up to 80% lower TTFT. A community user (Novita) reports hit rate rising from 40% to 80% and TTFT falling 56% [S7] **[>6mo, 2025-09-10]**.
- **Routing-key header.** SGLang's serving layer reads `x-smg-routing-key`, which feeds the `routing-key` schedule policy [S4].

**TensorRT-LLM** [T1][T2][T3].
- **KV cache reuse** is on by default (`enableBlockReuse=true`) and needs paged context FMHA.
- **`KvCacheRetentionConfig`** sets per-token-range priority (1–100, default 35) with optional duration via `TokenRangeRetentionConfig(start, end, priority, duration_ms)`, plus `decode_retention_priority`. This lets a caller pin a system prompt's blocks at high priority.
- **Conversation ID headers.** The serve layer reads, in order:
  `x-claude-code-session-id`, `x-claude-session-id`, `x-session-id`, `x-correlation-id`, `x-session-affinity`, `x-multi-turn-session-id`.
  The body field `conversation_params.conversation_id` takes precedence.
- **`x-trtllm-subagent-affinity-id`** is a "parent-session routing key" so that subagents route with the parent [T3].
- **`KvCacheAwareRouter`** routes on block hashes built from engine KV events. Conversation affinity keys on the first 256 tokens [T3].

### 3.2 KV offloading and sharing

- **LMCache** [C1][C2][C3].
  - A KV layer that sits outside the engine for vLLM and SGLang. Backends include CPU, local disk, Redis/Valkey, S3, Mooncake, NIXL, 3FS, Weka and others.
  - `chunk_size` defaults to 256 tokens. The hash is configurable (`pre_caching_hash_algorithm`, default "builtin"; the docs' example sets `PYTHONHASHSEED`).
  - Sharing works through a centralized cache server or P2P over NIXL with a controller.
  - The controller `lookup(tokens)` returns `{instance_id: (location, matched_prefix_length)}`. **An external coordinator could query where a prefix lives.**
  - The in-process mode is deprecated in favour of "MP mode".
  - The paper (Oct–Dec 2025) reports up to 15x throughput with vLLM, and that "context truncation … can greatly reduce prefix cache hit ratio by half" [P-lmcache].
  - CacheBlend reuses non-prefix chunks: TTFT 2.2–3.3x better and throughput 2.8–5x higher [P-cacheblend] **[>6mo]**.
- **Mooncake** [P-mooncake].
  - Kimi's KVCache-centric disaggregated architecture: separate prefill and decode clusters, and a disaggregated KV cache across the cluster's CPU, DRAM and SSD.
  - "Enables Kimi to handle 75% more requests" under real workloads, and up to +525% throughput in simulation **[>6mo, v4 2025-09]**.
  - The Mooncake store is a backend for SGLang HiCache and LMCache.
  - Its FAST'25 best-paper status: **UNVERIFIED**.
- **NVIDIA Dynamo** [N1][N3].
  - The KV router credits overlap with device, host, disk and shared-cache tiers.
  - Dynamo describes offloading KV blocks to host memory and storage during tool calls and prefetching them back.
  - The KVBM and NIXL internals were not read in detail (**UNVERIFIED** beyond these statements).

### 3.3 Cache-aware routing across replicas, and the hints a caller can pass

| Router | How it decides | Caller-supplied hints (exact names) |
|---|---|---|
| **llm-d** (v0.9; CNCF sandbox; EPP) [L1][L2][L3] | **Approximate:** characters approximate ~16-token blocks; a rolling hash chain; in-memory LRU index of hash → pod that learns from its own routing decisions. **Precise:** exact token IDs from vLLM `/v1/completions/render`; engines stream KVEvents over ZMQ into a global KV-Cache Indexer; "speculative indexing" fills the gap before events arrive; the prefix score is combined with load scorers. | `session-affinity-scorer` has two strategies. `encoded_endpoint_header` (default) uses header `x-session-token`, a base64 pod echoed back. `session_id` reads header `x-session-id` (configurable sources) and keeps a server-side binding with a 300 s TTL. The agentic north star builds a session graph "from external hints (e.g. Anthropic `cache_control`, OpenAI `prompt_cache_key`)" [L4]. |
| **Gateway API Inference Extension (GIE)** EPP [L5] | Proposal 0602 (implemented): approximate prefix index; character chunks; `hash(chunk_i) = hash(content_i + hash(chunk_{i−1}))`. Non-goals: no model-server cache API, no remote caches. Plugin parameters `hashBlockSize`, `maxPrefixBlocksToMatch`, `lruCapacityPerServer` and the CRD rename InferenceModel → InferenceObjective come from search summaries only (**UNVERIFIED**). | llm-d plugins (above) are built on the EPP. |
| **SMG (Shepherd Model Gateway)**, formerly SGLang router / sgl-model-gateway, v1.11.0 [S5][S6][S8] | `cache_aware` (default): a per-model approximate multi-tenant radix tree (string or token; token tree in `--block-size` pages, default 16), or, for gRPC workers, the engines' KV events. Falls back to an expected-wait score when the match is below `--cache-threshold` or the holder is overloaded (`--balance-abs-threshold`, `--balance-rel-threshold`). Also `prefix_hash` (hash the head at `--cache-boundaries`), `consistent_hashing`, `manual`, `least_load`, `power_of_two`, `bucket`. | `X-SMG-Routing-Key` (consistent hashing or manual pinning); `X-SMG-Target-Worker` (index); `--routing-key-override` sticky sessions using body `rid` with `_t<n>` / `_r<n>` suffixes stripped (`conv_t2_r1` → `conv`); **`x-smg-routing-tokens`** (≤512 leading token IDs, ≤4,096 bytes) to route without body parsing; cache partitions from `cache_salt`, `extra_key` and LoRA (HTTP workers only). |
| **NVIDIA Dynamo KV router** [N1][N2][N4] | Request tokens are hashed into blocks. A KvIndexer prefix tree is fed by worker KV events. Cost = `prefill_load_scale × max(0, active_prefill + incoming_prompt − overlap_credit) + potential_decode_blocks + decode_active_request_weight × active_requests`. Lowest cost wins; softmax sampling when `router_temperature > 0`. | **`X-Dynamo-Session-ID`**, **`X-Dynamo-Parent-Session-ID`**, **`X-Dynamo-Session-Final`** (end-of-session release). Native Claude Code headers (`x-claude-code-session-id`, `x-claude-code-agent-id`, `x-claude-code-parent-agent-id`), Codex (`thread-id`, `x-codex-parent-thread-id`) and OpenCode (`x-session-id`, `x-parent-session-id`) are recognized. **`nvext.agent_hints`**: `priority`, `strict_priority`, `osl` (expected output length), `speculative_prefill` (warm the predicted next-turn prefix). Session IDs alone "do not enable sticky sessions". |
| **AIBrix** [K1][K2] | Per-request `routing-strategy` header or `ROUTING_ALGORITHM`. `prefix-cache` uses a local hash table, or KV-sync mode with a distributed index (`AIBRIX_PREFIX_CACHE_KV_EVENT_SYNC_ENABLED=true`); the best prefix-matched pod within a stddev load threshold wins. `prefix-cache-preble` (Preble-based). `session-affinity`. | **`x-aibrix-session-key`** (caller-provided, works from the first request, intended for concurrent agentic requests); `x-session-id` (gateway-issued base64 pod address echoed back); `routing-strategy`. |
| **TensorRT-LLM router** [T3] | `KvCacheAwareRouter` matches block hashes against engine KV events; load-balancing and round-robin routers also exist. | Conversation-ID headers (§3.1), `x-trtllm-subagent-affinity-id`, `conversation_params.conversation_id`. |
| **vLLM production-stack router** [V-ps] | README lists `--routing-logic roundrobin` or `session` with `--session-key <header>`. The README may be stale; KV-aware or prefix-aware routing there is **UNVERIFIED**. | The header named by `--session-key`. |
| **OpenRouter** (hosted aggregator) [R1] | Sticky per account, model and conversation; the conversation is hashed from the first system and first non-system messages. | `session_id` (body) or `x-session-id` (≤256 chars). |

**Published routing gains.**
- **llm-d** [L1] **[>6mo, 2025-09-24]**. Setup: 8 vLLM pods on 16 H100s, 150 customers with 6,000-token contexts, 73% of cluster KV capacity.
  - Precise scheduling reached P90 TTFT **0.542 s**, against more than 31 s for approximate and more than 90 s for cache-blind schedulers. That is "57x faster than approximate" and "170x faster than random".
  - A ~10k-token prompt repeated on one vLLM instance dropped TTFT from 4.3 s to 0.6 s.
  - A "2x throughput vs load-aware" figure comes from the search summary (**UNVERIFIED**).
- **SGLang v0.4 cache-aware load balancer:** up to 1.9x throughput with 3.8x higher hit rate [S9] **[>6mo, 2024-12-04]**.
- **Preble:** 1.5–14.5x average latency and 2–10x p99 [P-preble] **[>6mo]**.

**Can an external coordinator pass a hint? Yes, on every major self-hosted router.** The workable common denominator is one stable session ID per reasoning chain, plus a parent ID for subagents. In practice:
- Maidan's thread or claim ID maps to `X-Dynamo-Session-ID`, `x-aibrix-session-key`, `X-SMG-Routing-Key`, `x-session-id` (llm-d, OpenRouter, TRT-LLM) or SGLang `session_id`.
- The parent thread maps to `X-Dynamo-Parent-Session-ID` or `x-trtllm-subagent-affinity-id`.
- An explicit end signal maps to `X-Dynamo-Session-Final` or SGLang `/close_session`.
- A tenant boundary maps to `cache_salt`.
- None of the routers accepts a "prefix hash" from the client directly. The closest is SMG's `x-smg-routing-tokens` (leading token IDs). Prefix matching is otherwise computed by the router from the body or from engine events.

---

## 4. Research and practice on multi-agent token waste and cache reuse

### 4.1 How much input is duplicated or cached in practice

| Source (date) | Finding |
|---|---|
| Anthropic, "How we built our multi-agent research system" (2025-06-13) [P-anth] **[>6mo]** | Agents use ~4x the tokens of chat; multi-agent systems ~15x. Token usage alone explains 80% of BrowseComp performance variance. The multi-agent system beat single-agent Opus 4 by 90.2%. |
| Manus, "Context Engineering for AI Agents" (2025-07-18) [P-manus] **[>6mo]** | "KV-cache hit rate is the single most important metric for a production-stage AI agent." Average input:output ~100:1. Practices: stable prefix (no second-precision timestamps), append-only context, deterministic serialization, explicit breakpoints, session IDs for vLLM routing, mask tools rather than remove them. |
| Cognition, "Don't Build Multi-Agents" (2025-06-12) [P-cog] **[>6mo]** | Principles: "share context"; "actions carry implicit decisions". |
| TraceLab (arXiv 2606.30560, 2026-06-29) [P-tracelab] | ~4,300 Claude Code and Codex sessions, 350k LLM steps. ~**96%** of prompt tokens served from prefix cache; median step is 119K prefix, 875 append and 214 output tokens; "prefix-cache reads dominate the overall API cost". Fresh tokens are only **19.0%** of appended tokens (12.3% Claude, 25.8% Codex), so ~81% of prefill could in principle be cached; **prefill amplification 5.3x** (8.1x Claude, 3.9x Codex). Misses cluster on user-initiated steps after idle >5 min; after 1 h almost all miss. Eviction sweep: 1 min → 85.4% hit, 5 min → ~94%, 1 h → 98.6%. |
| "Token Reduction Is Not Cost Reduction" (arXiv 2607.12161, 2026-07-13, v5 2026-08-12) [P-trnc] | Claude Code baseline: cache creation plus reads ≈ **87%** of reconstructed cost (≈80% of bill). **Harness base (system prompt + tool defs) = 71.6% of context, 92.0% of cache reads, 74.7% of cost.** Tool outputs were only 3.3% of cost. Compressing tool output by 38.4% *raised* billed cost by 6.8%; token reduction vs cost reduction r = 0.15. |
| "Don't Break the Cache" (arXiv 2601.06007, 2026-01-09) [P-dbtc] | 500+ DeepResearch Bench sessions with 10k-token system prompts across OpenAI, Anthropic and Google: caching cut cost **41–80%** and TTFT 13–31%. Strategic boundaries (dynamic content last, excluding dynamic tool results) beat naive full-context caching, which "can paradoxically increase latency". |
| "How Do AI Agents Spend Your Money?" (arXiv 2604.22750, 2026-04-24) [P-spend] | Agentic coding uses ~1000x more tokens than code chat; input drives cost; up to 30x variance between runs of the same task; Kimi-K2 and Sonnet 4.5 average >1.5M more tokens than GPT-5. |
| Tokenomics (arXiv 2601.14470, 2026-01-20) [P-tokenomics] | ChatDev with GPT-5: code review is 59.4% of tokens; input 53.9%. |
| TokenCast (arXiv 2609.35760, 2026-09-28) [P-tokencast] | Context re-read inflates every later call; budget control with forecasts uses 21.3% fewer tokens. |
| AgentPrune / "Cut the Crap" (arXiv 2410.02506, 2024-10) [P-agentprune] **[>6mo]** | Pruning inter-agent "communication redundancy": $5.6 vs $43.7 at comparable results; 28.1–72.8% fewer tokens. |
| LMCache, "Context Engineering & Reuse Pattern Under the Hood of Claude Code" (2025-12-23) [P-lmcache-cc] **[>6mo]** | One SWE-bench task: 92 LLM calls, ~2M input tokens, **92% prefix reuse**; 92–98% per phase (Explore, Plan, main). Warm-up calls pre-cache the tool list and the Explore and Plan subagents. Their cost estimate falls from ~$6.00 to $1.152 (−81%) with caching. |
| NVIDIA Dynamo, "Full-Stack Optimizations for Agentic Inference" (March 2026, updated 2026-06-12) [N3] | Claude Code: after the first call, each later call to the same worker hits 85–97% cache; 11.7x read/write ratio. Agent teams: 97.2% aggregate hit across 4 Opus teammates; **teammates 79.4% vs 91.3% for explore subagents (5.0x vs 11.7x read/write), "driven almost entirely by cold-start writes on each teammate's first call."** Value by block type: system plus tools highest; history high; reasoning and dead-subagent KV near zero. |
| MAST, "Why Do Multi-Agent LLM Systems Fail?" (arXiv 2503.13657) [P-mast] **[>6mo]** | 1,600+ traces, 14 failure modes; no token-waste numbers. |

### 4.2 KV-cache sharing across agents and non-prefix reuse

Almost all of this needs control of the inference server. Only the application-level items marked hosted-OK work through hosted APIs.

| Work (date, venue) | Mechanism | Headline numbers | Hosted-API applicable? |
|---|---|---|---|
| **KVCOMM** (2025-10, NeurIPS 2025) [P-kvcomm] | Training-free cross-context KV reuse between agents; an "anchor" pool estimates offset deviations under different prefixes | 70–87.6% reuse; up to 7.8x prefill speedup; TTFT ~430 → ~55 ms (5 agents, 1K input) | No |
| **DroidSpeak** (2024-11 → 2025-07) [P-droid] | Reuse KV across *different* fine-tuned LLMs of the same architecture; recompute a few layers | up to 4x throughput; ~3.1x faster prefill; negligible quality loss | No |
| **CacheBlend** (2024-05 → 2025-04) [P-cacheblend] | Fuse precomputed non-prefix chunk KV; selectively recompute a small subset of tokens | TTFT 2.2–3.3x; throughput 2.8–5x | No (in LMCache) |
| **EPIC** (2024-10) [P-epic] | Position-independent caching ("LegoLink") | up to 8x TTFT, 7x throughput | No |
| **Prompt Cache** (MLSys 2024) [P-promptcache] | Schema-declared "prompt modules" with precomputed attention states | TTFT 8x (GPU), 60x (CPU) | No (conceptually like explicit breakpoints) |
| **KVLink** (2025-02) [P-kvlink] | Per-document KV, positional fix plus trainable link tokens | TTFT −96%; +4% QA accuracy | No (needs training) |
| **Block-Attention** (ICLR 2025) / **TurboRAG** (2024-10) / **APE** (ICLR 2025) | Block-wise or parallel encoding, mostly with fine-tuning | TTFT −98.7% / 9.4x / 4.5x end-to-end | No |
| **Cache-Craft** (SIGMOD 2025) | Chunk-cache management for RAG | −51% redundant compute vs prefix caching | No |
| **KVFlow** (2025-07) [P-kvflow] | Workflow-aware eviction from an "Agent Step Graph" (steps-to-execution) plus CPU→GPU prefetch | 1.83x (single workflow), 2.19x (many concurrent) vs SGLang HiCache | No, but **the step graph is exactly what a coordinator knows** |
| **Continuum** (2025-11, v7 2026-09-08) [P-continuum] | KV TTL pinning during tool calls, set from reload cost and queueing | >8x average job completion time on SWE-Bench, BFCL and OpenHands | No |
| **TokenCake** (EuroSys '27) [P-tokencake] | Offload idle agent KV during function calls; reserve memory for critical-path agents | −47.06% latency; +16.9 pp GPU utilization | No |
| **Autellix / Agentix** (2025-02; NSDI '26) [P-autellix] | Program-level scheduling (treat agent programs as first-class) | 4–15x program throughput | No, but program IDs map to session IDs |
| **Parrot** (OSDI 2024) [P-parrot] | "Semantic Variables" expose cross-request dataflow to the server | up to an order of magnitude | Only if the provider adopted it |
| **InferCept** (2024) | Keep KV across tool interceptions | recomputation was 37–40% of forward time; 1.6–2x throughput | No |
| **Pie** (SOSP 2025) | Programmable "inferlets" (WASM) controlling KV | 1.3–3.4x on agentic workflows | No |
| **SGLang** (2023/24) / **Hydragen** (2024) / **ChunkAttention** (ACL 2024) | Radix prefix sharing; shared-prefix attention kernels | 6.4x; up to 32x; 3.2–4.8x kernel | No |
| **CachedAttention / AttentionStore** (ATC 2024) / **MemServe** (2024) / **Marconi** (MLSys 2025) | Hierarchical multi-turn KV; global prompt-tree scheduling; hybrid-model prefix caching | TTFT −87%, cost −70%; —; 34.4x token hit rate | No |
| **Agentic Plan Caching** (NeurIPS 2025) [P-apc] | Cache and adapt *plan templates* across similar tasks (application level) | −50.31% cost, −27.28% latency | **Yes (hosted-OK)** |
| **Cortex** (2025-09) [P-cortex] | Semantic knowledge cache for agent remote data access | 3.6x throughput at >85% hit | **Yes (app level)** |
| **Auditing Prompt Caching in LM APIs** (2025-02, ICML 2025) [P-audit] **[>6mo]** | Timing-side-channel audits | Found **global cross-user cache sharing in seven API providers, including OpenAI** (at the time). Current OpenAI docs say caches are not shared across organizations [O1] (**conflict / likely changed since**). Relevant to Maidan's tenant model. | n/a |

Most of the 2023–2025 papers in this table are **[>6mo]**. Their dates are the arXiv first-submission dates; see §8.

### 4.3 Scheduling policies shown to raise hit rates

- **Longest-prefix-match and DFS ordering** within a server queue (SGLang `lpm`, `dfs-weight`), plus in-batch prefix dedup [S4][S1].
- **Prefix-affine placement across replicas, with load spill.** All routers in §3.3 do this; Preble co-optimizes reuse against load [P-preble].
- **Warm-then-fan-out:**
  - Anthropic: wait for the first response before parallel requests [A1]; batch recipe of 1h prefix first, then the rest [A1];
  - Claude Code warm-up calls [P-lmcache-cc];
  - Dynamo `speculative_prefill` [N1];
  - OpenAI `prewarm` [O1];
  - Anthropic `max_tokens:0` [A1].
- **Retention by future use rather than recency:** KVFlow steps-to-execution, Continuum TTL during tool calls, SGLang `priority` and `tlru`, TRT-LLM token-range priorities, SGLang session references [P-kvflow][P-continuum][S2][T2][S3].
- **Keep idle sessions warm across human-paced gaps** (TraceLab: most remaining misses occur after idle >5 min) [P-tracelab].

---

## 5. Implications for a coordinator in Maidan's position

Maidan does not call LLMs. What it controls is:
- **what context it hands out, and in what byte layout**, through context packs and content-addressed snapshots;
- **when and to whom work is released**, through threads and claims;
- **what metadata travels with a claim** (IDs, deadlines, parent/child), and the shared event log.

Agents and harnesses choose the provider account, the model and the cache markers.

### 5.1 What a coordinator can do that a single agent cannot

1. **Canonical, cache-ordered context packs.**
   - Emit packs as byte-stable segments ordered from most to least shared:
     `[harness base] → [org/workspace pack] → [thread pack] → [task suffix]`.
   - Use deterministic serialization: sorted JSON keys, no timestamps, LSNs or request IDs in cacheable segments. Every provider treats a one-token change as a miss from that point on [A1][O1][P-manus].
   - Ship a `cache_plan` with each pack:
     - segment content hashes and token estimates per tokenizer family;
     - recommended breakpoint positions (Anthropic ≤4; OpenAI ≤4 writes per request);
     - TTL suggestion;
     - whether the segment clears each model's minimum (512–4,096 / 1,024 / 2,048–6,144).
   - Content-addressed snapshots already give Maidan stable hashes for this.
2. **Put shared packs early, not in mid-session tool results** (inference from [A1][O1]).
   - A pack delivered as an MCP tool result lands at a different message position in each agent's history. So it is never a shared prefix across agents, only within one agent's conversation.
   - To share across agents, deliver a "boot pack" *before* the session starts, via REST or CLI, and inject it as a system append or the first user message, right after an identical harness base.
3. **Freeze Maidan's own MCP tool surface** (inference from [A1][O1]).
   - Tool definitions sit at the *front* of the prefix. Any change to Maidan's `tools/list` (names, descriptions, schema, ordering) invalidates every connected agent's whole cache.
   - Capability-filtered tool lists that differ per member or role also split agents into separate prefix groups.
   - Version tool descriptions deliberately, keep the order deterministic, and prefer one stable list with server-side authorization over per-role lists. Where tools must change, prefer append-only lists, or `allowed_tools` / `tool_choice:"none"` over editing the list [O3], or Anthropic's `inline-tools` beta [A1].
4. **Warm-then-fan-out scheduling.**
   - When N claims share a pack, release one "warmer" claim first, or have the harness pre-warm (Anthropic `max_tokens:0`, OpenAI `prewarm:true`). Release the rest once the first response has *started* [A1].
   - Worked example (derived from [A2]): 8 agents, shared 40k-token prefix, Sonnet 5.5.

     | Strategy | Cost of the shared prefix |
     |---|---|
     | Parallel cold | 8 × $0.10 write = **$0.80** |
     | No caching | 8 × $0.08 = **$0.64** |
     | Warm-then-fan-out | $0.10 + 7 × $0.008 = **$0.156** (5.1x cheaper than parallel cold) |

   - On gpt-5.6-sol (derived from [O2]): parallel cold $1.60 vs warm-then-fan-out $0.312.
   - Measured analogue: Claude Code teammates lost about 12 points of hit rate to cold starts [N3].
5. **TTL-aware claim timing and keep-alive decisions.**
   - Maidan sees idle gaps: human approvals, held gates, tool waits. It can (a) schedule follow-ups inside the TTL, (b) ask the harness to choose the 1h TTL, or (c) send keep-alive reads.
   - Derived rule of thumb for Anthropic:
     - 5m TTL with pings every ~4.5 min costs `1.25 + 0.1·n` (prefix units). A 1h write costs `2.0`. 5m plus pings is cheaper for gaps up to about 34 minutes.
     - On Opus 5.5 (0.05x reads) the break-even moves to about 67 minutes, past the 1h horizon.
   - Two inferences behind this, both **UNVERIFIED**: that a `max_tokens:0` pre-warm hitting an existing entry counts as a refreshing read; and that pings still cost a request and their uncached suffix.
   - TraceLab's sweep (5 min → ~94% hit, 1 h → 98.6%) bounds the upside [P-tracelab].
6. **Prefix-group affinity keys.**
   - Assign one stable key per shared-prefix group (not per agent) and hand it to harnesses:
     - `prompt_cache_key` for OpenAI before 5.6, Azure, Mistral and xAI Responses;
     - `x-grok-conv-id` for xAI;
     - `session_id` / `x-session-id` for OpenRouter;
     - one shared DeepSeek `user_id` per sharing group.
   - Shard deterministically above ~15 RPM per key on OpenAI and Azure [O1][Z1].
   - Use distinct keys or `cache_salt` *across tenants* to block cache-hit probing [O1][V-vllm][P-audit].
7. **Model, tier and region pinning per prefix group.** Caches are per model [A3] and, at OpenAI, also per service tier and processing region [O1][O3]. A claim router that picks models or tiers per claim should keep each prefix group on one (model, tier, region, account) tuple. Do not mix flex and standard traffic on one OpenAI prefix group.
8. **Discount lanes driven by claim deadlines.**
   - **Batch:** Anthropic stacks 50% with cache; use the 1h warm request first [A1][A2]. OpenAI GPT-5.x/6.x batch cached rates apply [O2]. Gemini batch with `cached_content` [G5]. *Not* Bedrock batch, which has no caching [B1]. xAI batch is only 20% on some models [X2].
   - **Flex for synchronous agent loops:** OpenAI flex at batch prices [O7]; Gemini Flex at 50% with a 1–15 min target [G6]; Bedrock Flex at 50% [B2].
   - **Off-peak time-shift:** DeepSeek at 50% outside 01:00–04:00 and 06:00–10:00 UTC on weekdays [D2].
9. **One normalized ledger plus cache diagnostics in the event log.**
   - Store per call: `uncached_input`, `cache_read`, `cache_write{ttl}`, `output`, `reasoning`, `storage_token_hours` (Gemini explicit), `provider`, `scope_id` (workspace/org/project/subscription/account), `model`, `service_tier`, `region`, `batch`, `cache_key`, `pack_id` / `prefix_hash`, and `previous_response_id`.
   - Apply the per-provider mappings in §1.13. Mind the "input includes cached?" difference.
   - KPIs: token hit rate = read / (uncached + read + write); write/read ratio (Dynamo uses 11.7x as its reference [N3]); prefill amplification (TraceLab) [P-tracelab].
   - Capture Anthropic `diagnostics.cache_miss_reason` and OpenAI `prompt_cache_diagnostics.reason` by having harnesses pass the previous response ID [A3][O3]. This turns "which agent broke the prefix" into an event-log query.
10. **Harness hygiene profiles.**
    - Claude Code's cache is "effectively scoped to one machine and directory" (working directory, OS and git-status snapshot in the conversation; auto-memory path in the system prompt) [A9].
    - The Agent SDK's `excludeDynamicSections: true` / `exclude_dynamic_sections` moves per-user context out of the system prompt so fleets share a system-prompt cache entry [A10].
    - Maidan can publish and check profiles: identical mount paths, an SDK flag, a fixed git snapshot point per thread.
    - **Fork over spawn** where context should be shared: a Claude Code fork inherits the parent prefix and reads its cache, while a subagent with a different system prompt starts cold [A9]. Express role differences as late suffixes rather than different system prompts, where that is compatible with role isolation.
11. **Self-hosted hint emission** (optional gateway or SDK shim). Map thread and claim structure onto the hints in §3.3:
    - session ID per reasoning chain, parent ID per subagent, end-of-session signal;
    - priorities from Maidan's critical path;
    - `speculative_prefill` before the next claim is released;
    - TRT-LLM retention priority for pack token ranges.

    KVFlow's "steps-to-execution" is a function of the claim DAG Maidan already holds [P-kvflow]. LMCache's controller lookup [C3] or llm-d's indexer [L2] could tell Maidan where a pack's KV already lives, so it can release work there.

### 5.2 Hard limits

- **Scope.** Sharing is only possible inside one provider boundary:
  - Anthropic: workspace on the Claude API, Platform on AWS and Foundry; organization on Bedrock and Vertex [A1];
  - OpenAI: organization plus processing region, and further split by `prompt_cache_key` [O1];
  - Azure: subscription [Z1];
  - Gemini: project, and region for explicit caches [V3];
  - DeepSeek: account, partitioned by `user_id` [D3].

  Agents with keys from different organizations or workspaces (BYOK, multi-tenant) never share, whatever Maidan does. Maidan sees no keys, so it can recommend placement but cannot enforce it.
- **Model-scoped, exact-prefix only on hosted APIs.** There is no reuse across models, versions, providers or service tiers, and no non-prefix reuse (KVCOMM, CacheBlend). The research gains in §4.2 are self-hosted only.
- **Minimum lengths and write premiums.**
  - A pack segment below 512–6,144 tokens (provider- and model-dependent) gets nothing.
  - With write premiums (Anthropic 1.25x/2x; OpenAI 5.6+, Bedrock GPT-5.6, Alibaba and Kimi on Bedrock 1.25x), a miss-heavy pattern costs *more* than no caching.
  - OpenAI's padding break-even formula shows when to pad a segment up to the minimum [O1].
- **Cold writes under concurrency.** The Anthropic entry appears only after the first response begins [A1]. OpenAI caches live per machine and overflow above ~15 RPM per key [O1]. xAI caches are per server and evictable at any time [X1]. Fan-out without warming always pays N writes.
- **TTL and eviction are not controllable on most hosted APIs.** Anthropic offers 5m/1h; OpenAI 5.6+ only "30m" minimum; the rest are best-effort. Human-paced gaps beyond 5 minutes are where caches die [P-tracelab]. No provider exposes residency or lets you pin or clear entries; you only see usage after the fact.
- **Rate-limit semantics differ.** Anthropic cache reads don't count toward ITPM [A5]. OpenAI cached tokens do count [O1]. Gemini limits include cached tokens [G2]. Bedrock-hosted GPT-5.6 cached tokens don't count [B1]. The throughput benefit of caching is therefore provider-specific.
- **Privacy.** Sharing caches across trust boundaries creates timing side channels [P-audit]. Maidan's tenant boundaries must also be cache boundaries (separate keys or `cache_salt`), which is consistent with its two-tenant testing discipline.
- **Diagnostics are narrow.** Anthropic's works on the Claude API only (not Bedrock, Vertex or Platform on AWS) [A3]; OpenAI's on Responses with GPT-5.6+ only [O3]. Both compare against one earlier response and expire quickly.
- **Harness cooperation is required.** Maidan cannot place `cache_control`, choose TTLs, set `prompt_cache_key` or send pre-warms itself. Every lever needs the harness or SDK, or an opt-in LLM gateway, to act on Maidan's `cache_plan`.

### 5.3 A program sketch (ordered by leverage)

| Phase | Work | Basis |
|---|---|---|
| P0 Measure | Normalized ledger (§1.13); per-pack and per-prefix-group hit rate, write/read ratio, amplification; capture diagnostics reasons into the event log | [A1][A3][O1][O3][P-tracelab] |
| P1 Shape | Cache-ordered, byte-stable packs with a `cache_plan`; boot-pack injection instead of mid-session tool results; frozen and versioned MCP tool list; harness profiles (`excludeDynamicSections`, fixed paths) | [A1][A9][A10][P-trnc] |
| P2 Schedule | Warm-then-fan-out release of claims; TTL-aware timing and keep-alive vs 1h choice; prefix-group keys; model, tier and region pinning | [A1][O1][N3] |
| P3 Discount lanes | Claim deadlines route to batch, flex or off-peak lanes with cache-aware recipes | [A4][O6][O7][G5][G6][D2] |
| P4 Self-hosted hints | Optional gateway emitting session, parent, final, priority and speculative-prefill hints; DAG-aware retention; KV-location-aware release | [N1][N2][S3][T2][T3][K2][L3][C3][P-kvflow] |

---

## 6. Conflicts and staleness flags

1. **Opus 4.7 minimum:** 2,048 per Anthropic [A1] vs 4,096 per Bedrock [B1].
2. **Gemini minimum:** 4,096 for 3.x Flash and 3.1 Pro Preview per the Gemini API [G2] vs 6,144 (implicit-only) per Vertex [V2].
3. **Gemini batch cache price:** batch doc "standard context caching rates" [G5] vs pricing table batch cache price at 50% [G3].
4. **OpenAI lookup window:** guide "first 2 and latest 50 explicit breakpoints + 20 earlier message endings" [O1] vs API reference "latest 80 breakpoints, no content-block lookback limit" [O4].
5. **`prompt_cache_key` on GPT-5.6+:** not needed for routing (OpenAI) [O1] vs "improves cache matching", ~15 RPM per key (Azure) [Z1].
6. **Cached tokens vs rate limits:** OpenAI direct counts them [O1]; GPT-5.6 on Bedrock doesn't [B1]; Anthropic doesn't (most models) [A5]; Gemini includes them [G2].
7. **Vertex Claude page:** "90% cheaper" reads [V1] vs Anthropic's 0.05x (Opus 5.5) and 0.025x (Fable 5.1) [A2]. Possibly stale, or Google pricing differs.
8. **Vertex Claude scope:** "per project" [V1] vs "organization-level" (Anthropic) [A1]. Likely the same thing; **UNVERIFIED**.
9. **OpenRouter multipliers:** Google 0.25 [R1] vs Gemini 0.1x today [G3]. Grok varies 0.15–0.25x by model [X2].
10. **Cross-user cache sharing:** the Feb-2025 audit found it at OpenAI and six others [P-audit] **[>6mo]**; current OpenAI docs say no cross-org sharing [O1].
11. **Dynamo docs:** `docs.nvidia.com/dynamo/latest/...` returned 404 / "Page Not Found". Dynamo facts come from the docs on GitHub `main` (fern pages) [N1][N2][N4].
12. **vLLM production-stack router README:** may be stale [V-ps].
13. **Sources older than 6 months:** Anthropic multi-agent post, Manus, Cognition, LMCache Claude-Code blog (Dec 2025), SGLang v0.4 and HiCache blogs, llm-d blog (Sep 2025), and most papers in §4.2. Their *mechanisms* are likely still valid; their *numbers* reflect older models and prices.

## 7. UNVERIFIED items

- Mistral cache TTL and scope; xAI minimum length, scope and write fees; DeepSeek minimum length and whether hit plus miss equals `prompt_tokens`.
- Gemini API implicit caching TTL and scope (API key vs project); full Interactions API usage fields.
- Bedrock cache scope as AWS states it (account / region); usage-field names on Bedrock InvokeModel and the new `/anthropic/v1/messages` endpoint; Bedrock flex/priority interaction with caching.
- Azure flex/priority equivalents and batch-plus-cache discount stacking; Mistral batch-plus-cache stacking.
- Whether OpenAI caches are shared across projects within an organization.
- Whether Anthropic real-time and Batch traffic share cache entries within a workspace.
- Whether an Anthropic `max_tokens:0` request that hits an existing entry refreshes its TTL (assumed in §5.1.5).
- GIE plugin parameter names and InferenceObjective CRD details (search summary only); llm-d "2x throughput vs load-aware" (search summary only).
- Dynamo KVBM / NIXL internals; Mooncake FAST'25 best-paper status; vLLM production-stack KV-aware routing.
- OpenRouter's claimed 82.8% platform hit rate (secondary blog).

## 8. Source index (all retrieved 2026-09-30/2026-10-01)

**Anthropic**
- [A1] https://platform.claude.com/docs/en/build-with-claude/prompt-caching (Markdown `.md` version)
- [A2] https://platform.claude.com/docs/en/about-claude/pricing
- [A3] https://platform.claude.com/docs/en/build-with-claude/cache-diagnostics
- [A4] https://platform.claude.com/docs/en/build-with-claude/batch-processing
- [A5] https://platform.claude.com/docs/en/api/rate-limits
- [A6] https://platform.claude.com/docs/en/api/service-tiers
- [A7] https://platform.claude.com/docs/en/build-with-claude/claude-in-amazon-bedrock
- [A8] https://platform.claude.com/docs/en/build-with-claude/claude-platform-on-aws
- [A9] https://code.claude.com/docs/en/prompt-caching
- [A10] https://code.claude.com/docs/en/agent-sdk/modifying-system-prompts

**AWS and Google Cloud**
- [B1] https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-caching.html
- [B2] https://aws.amazon.com/bedrock/pricing/
- [V1] https://docs.cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude/prompt-caching (page "Last updated 2026-10-01 UTC")
- [V2] https://docs.cloud.google.com/vertex-ai/generative-ai/docs/context-cache/context-cache-overview
- [V3] https://docs.cloud.google.com/vertex-ai/generative-ai/docs/context-cache/context-cache-create

**OpenAI and Azure**
- [O1] https://developers.openai.com/api/docs/guides/prompt-caching (`.md`)
- [O2] https://developers.openai.com/api/docs/pricing (`.md`)
- [O3] https://developers.openai.com/api/docs/guides/prompt-caching/diagnostics
- [O4] https://developers.openai.com/api/reference/resources/responses/methods/create
- [O5] https://developers.openai.com/api/reference/resources/chat
- [O6] https://developers.openai.com/api/docs/guides/batch
- [O7] https://developers.openai.com/api/docs/guides/flex-processing
- [O8] https://developers.openai.com/api/docs/guides/fast-mode
- [Z1] https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/prompt-caching
- [Z2] https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/batch

**Google Gemini API**
- [G1] https://ai.google.dev/gemini-api/docs/caching
- [G2] https://ai.google.dev/gemini-api/docs/generate-content/caching
- [G3] https://ai.google.dev/gemini-api/docs/pricing
- [G4] https://ai.google.dev/api/generate-content (UsageMetadata)
- [G5] https://ai.google.dev/gemini-api/docs/batch-api
- [G6] https://ai.google.dev/gemini-api/docs/flex-inference
- [G7] https://ai.google.dev/gemini-api/docs/priority-inference

**DeepSeek, Mistral, xAI, OpenRouter**
- [D1] https://api-docs.deepseek.com/guides/kv_cache
- [D2] https://api-docs.deepseek.com/quick_start/pricing
- [D3] https://api-docs.deepseek.com/quick_start/rate_limit
- [M1] https://docs.mistral.ai/studio-api/conversations/advanced/prompt-caching
- [M2] https://docs.mistral.ai/studio-api/batch-processing
- [X1] https://docs.x.ai/developers/advanced-api-usage/prompt-caching (plus subpages `/how-it-works`, `/maximizing-cache-hits`, `/usage-and-pricing`, `/best-practices`)
- [X2] https://docs.x.ai/developers/pricing
- [R1] https://openrouter.ai/docs/guides/best-practices/prompt-caching

**vLLM**
- [V-vllm] https://docs.vllm.ai/en/latest/design/prefix_caching.html (repo `docs/design/prefix_caching.md`); code `vllm/config/cache.py`, `vllm/distributed/kv_events.py`, `vllm/v1/metrics/loggers.py`, `vllm/entrypoints/openai/` on GitHub `main`
- [V-ps] https://github.com/vllm-project/production-stack/blob/main/src/vllm_router/README.md

**SGLang and SMG**
- [S1] https://arxiv.org/abs/2312.07104 (SGLang)
- [S2] SGLang docs "Radix Cache Eviction Policies" (sgl-project/sglang `docs/`, GitHub `main`)
- [S3] SGLang docs "Session-Aware Radix Cache" (same repo)
- [S4] SGLang server arguments doc, plus `python/sglang/srt/managers/schedule_policy.py` and `serving_base.py` (GitHub `main`)
- [S5] https://lightseek.org/smg/concepts/routing/load-balancing/
- [S6] https://lightseek.org/smg/concepts/routing/cache-aware/ and https://lightseek.org/smg/concepts/routing/sticky-sessions/
- [S7] https://www.lmsys.org/blog/2025-09-10-sglang-hicache/
- [S8] https://github.com/smg-project/smg (README)
- [S9] https://www.lmsys.org/blog/2024-12-04-sglang-v0-4/

**TensorRT-LLM**
- [T1] TensorRT-LLM docs "KV cache reuse" (NVIDIA/TensorRT-LLM `docs/`)
- [T2] TensorRT-LLM docs "How to Change Block Priorities" (KvCacheRetentionConfig)
- [T3] NVIDIA/TensorRT-LLM `tensorrt_llm/serve/conversation_id.py` and `tensorrt_llm/serve/router.py` (GitHub `main`)

**LMCache**
- [C1] https://github.com/LMCache/LMCache/blob/dev/docs/source/api_reference/configurations.rst
- [C2] https://github.com/LMCache/LMCache/blob/dev/docs/source/kv_cache/p2p_sharing.rst (and `getting_started/quickstart/share_kv_cache.rst`)
- [C3] https://github.com/LMCache/LMCache/blob/dev/docs/source/kv_cache_management/lookup.rst

**NVIDIA Dynamo**
- [N1] https://github.com/ai-dynamo/dynamo/blob/main/docs/fern/pages/use-cases/agents/agent-hints.md
- [N2] https://github.com/ai-dynamo/dynamo/blob/main/docs/fern/pages/use-cases/agents/session-ids.mdx
- [N3] https://github.com/ai-dynamo/dynamo/blob/main/docs/fern/pages/blog/2026/agentic-inference-optimizations.mdx (March 2026, last-updated 2026-06-12)
- [N4] https://github.com/ai-dynamo/dynamo/blob/main/docs/fern/pages/developer-guide/knowledge-base/modular-components/router/routing-concepts.md

**llm-d and GIE**
- [L1] https://llm-d.ai/blog/kvcache-wins-you-can-see (2025-09-24)
- [L2] https://llm-d.ai/docs/architecture/advanced/kv-management/prefix-cache-aware-routing (v0.9)
- [L3] https://github.com/llm-d/llm-d-router/tree/main/pkg/epp/framework/plugins/scheduling/scorer/sessionaffinity (README); https://llm-d.ai/docs/dev/architecture/core/router/epp/scheduling
- [L4] https://llm-d.ai/docs/dev/well-lit-paths/workloads/agentic-serving
- [L5] https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/main/docs/proposals/0602-prefix-cache-aware-routing-proposal/README.md

**AIBrix**
- [K1] https://github.com/vllm-project/aibrix/blob/main/docs/source/designs/aibrix-router.rst
- [K2] https://github.com/vllm-project/aibrix/blob/main/docs/source/features/agentic-routing.rst

**Papers and posts** (arXiv first-submission date in parentheses)
- [P-anth] https://www.anthropic.com/engineering/multi-agent-research-system (2025-06-13)
- [P-manus] https://manus.im/blog/Context-Engineering-for-AI-Agents-Lessons-from-Building-Manus (2025-07-18)
- [P-cog] https://cognition.ai/blog/dont-build-multi-agents (2025-06-12)
- [P-lmcache-cc] https://blog.lmcache.ai/en/2025/12/23/context-engineering-reuse-pattern-under-the-hood-of-claude-code/ (2025-12-23)
- [P-tracelab] https://arxiv.org/abs/2606.30560 (2026-06-29)
- [P-trnc] https://arxiv.org/abs/2607.12161 (2026-07-13, v5 2026-08-12)
- [P-dbtc] https://arxiv.org/abs/2601.06007 (2026-01-09)
- [P-spend] https://arxiv.org/abs/2604.22750 (2026-04-24)
- [P-tokenomics] https://arxiv.org/abs/2601.14470 (2026-01-20)
- [P-tokencast] https://arxiv.org/abs/2609.35760 (2026-09-28)
- [P-agentprune] https://arxiv.org/abs/2410.02506 (2024-10-03)
- [P-mast] https://arxiv.org/abs/2503.13657 (2025-03-17)
- [P-kvcomm] https://arxiv.org/abs/2510.12872 (2025-10-14, NeurIPS 2025)
- [P-droid] https://arxiv.org/abs/2411.02820 (2024-11-05)
- [P-cacheblend] https://arxiv.org/abs/2405.16444 (2024-05-26)
- [P-epic] https://arxiv.org/abs/2410.15332 (2024-10-20)
- [P-promptcache] https://arxiv.org/abs/2311.04934 (2023-11-07, MLSys 2024)
- [P-kvlink] https://arxiv.org/abs/2502.16002 (2025-02-21)
- [P-kvflow] https://arxiv.org/abs/2507.07400 (2025-07-10)
- [P-continuum] https://arxiv.org/abs/2511.02230 (2025-11-04)
- [P-tokencake] https://arxiv.org/abs/2510.18586 (2025-10-21, EuroSys '27)
- [P-autellix] https://arxiv.org/abs/2502.13965 (2025-02-19) and https://www.usenix.org/conference/nsdi26/presentation/luo
- [P-parrot] https://arxiv.org/abs/2405.19888 (2024-05-30)
- [P-preble] https://arxiv.org/abs/2407.00023 (2024-05-08)
- [P-mooncake] https://arxiv.org/abs/2407.00079 (2024-06-24)
- [P-lmcache] https://arxiv.org/abs/2510.09665 (2025-10-08)
- [P-apc] https://arxiv.org/abs/2506.14852 (2025-06-17)
- [P-cortex] https://arxiv.org/abs/2509.17360 (2025-09-22)
- [P-audit] https://arxiv.org/abs/2502.07776 (2025-02-11, ICML 2025)
- Other papers in §4.2: arXiv 2402.01869 (InferCept), 2402.05099 (Hydragen), 2402.15220 (ChunkAttention), 2403.19708 (CachedAttention), 2406.17565 (MemServe), 2411.19379 (Marconi), 2409.15355 (Block-Attention), 2410.07590 (TurboRAG), 2502.05431 (APE), 2502.15734 (Cache-Craft), 2510.24051 (Pie), 2310.07240 (CacheGen), 2404.12457 (RAGCache).
