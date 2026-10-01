# Context economics

The goal: a team of agents coordinated through Maidan pays for what changed,
not for what the room already knows, and can say what each finished task cost.

Maidan makes no model calls and holds no provider keys. It decides three
things an agent's bill depends on: the bytes it hands an agent, when work is
released, and what gets recorded about the cost. This program makes each of
them work with the providers' caches and discounts, and measures the result
per completed task.

The research behind it was done on 2026-10-01: a code audit of Maidan's
context surfaces, a survey of provider caching, batch and self-hosted engines,
and a survey of agent harnesses, the MCP and A2A specs, and the market. Prices
and limits below are as of that date and change often. Re-check them before
quoting them.

## What the providers reward

- **Exact prefixes.** Every hosted cache (Anthropic, OpenAI, Gemini, Bedrock,
  Azure, DeepSeek, Mistral, xAI) reuses a byte-identical prefix. Anthropic
  renders tools, then the system prompt, then messages, and one changed byte
  invalidates everything after it.
- **Cheap reads, priced writes.**
  - Anthropic: reads cost 0.1x of input (0.05x on Opus 5.5, 0.025x on Fable 5.1). Writes cost 1.25x for the 5-minute TTL and 2x for the 1-hour TTL.
  - OpenAI GPT-5.6 and later: writes cost 1.25x and reads 0.1x, with a TTL of at least 30 minutes.
  - Gemini: reads cost 0.1x, with no write premium, but explicit caches charge storage by the hour.
- **Narrow scope.** A cache never crosses provider accounts or models. It is per workspace on the Claude API and per organization on OpenAI, and on OpenAI also per service tier and region. Two agents share an entry only when they use the same account, model and prefix.
- **A cold start costs everyone.** An entry is readable only after the first response that writes it starts streaming. N agents that start together on the same context all pay the write.
- **Batch and flex.** Batch is 50% off at Anthropic, OpenAI and Gemini, and stacks with cache pricing. Bedrock's batch does not cache.

## Where the money goes, and where it does not

- Per agent, caching already works. Across about 4,300 Claude Code and Codex
  sessions, 95.7% of prompt tokens were served from cache (TraceLab, 2026). A
  pitch built on hit rate claims what the harnesses already deliver.
- The waste a coordinator can see is elsewhere:
  - cold writes when a team starts at once (teammates averaged 79.4% hits, against 91.3% for subagents, "driven almost entirely by cold-start writes");
  - the harness's own prefix (tools and system prompt were 74.7% of reconstructed cost in one study);
  - whole runs that duplicate or abandon work;
  - re-reading context that has not changed;
  - urgent-priced calls for work with slack.
- Token counts are not cost. One compressor cut tokens by 38.4% and raised the
  bill by 6.8%, because it broke prefixes.

## Where Maidan stood on 2026-10-01

- **The context pack was not cache-stable.**
  - REST and MCP returned different bytes for the same state.
  - Artifacts, references and decisions had no tiebreaker when their timestamps tied.
  - The volatile thread state sat near the top, and the stable grounding below the messages.
  - The token-budget fold rewrote the pack on every new message.
- **`as_of` was not exact.** It returned the live thread row.
- **`tools/list` was large and unstable.**
  - It carried 200 tools, about 93 KB or 25 to 28 thousand tokens, in every request of harnesses that do not defer tools.
  - It was filtered by each token's capabilities, so agents with different capabilities could not share a prefix.
  - It changed in most releases.
- **Maidan was out of step with MCP 2026-07-28.** It advertised that version but omitted `ttlMs` and `cacheScope`, which the schema requires on every cacheable result, and `server/discover`, which is a MUST.
- **The usage ledger was half there.**
  - It recorded cache-read and cache-write tokens, but nothing read them back.
  - Nothing said whether `input` includes cached tokens. Anthropic and Bedrock report it without them, and the others with.
  - One write tier could not price a mix of 5-minute and 1-hour writes.
  - It had no provider or evidence fields.
- **Nothing in the docs mentioned provider caching.**

## Principles

1. **Bytes are a contract.** The same state gives the same bytes on every
   surface (REST, MCP, the SDKs and snapshots), and a golden test says so.
2. **Stable first, volatile last, growth by appending.** A new message changes
   the end of a pack, not its middle.
3. **Boundaries a harness can use.** A cache breakpoint can only sit on a
   content-block boundary, so packs come in layers, each with its sha256.
4. **Shared bytes go first.** A pack returned by a tool lands at a different
   point in each agent's history, so agents never share it. Anything a team
   should share is a boot pack that the harness puts at the start of the
   session.
5. **Measure per completed task.** The headline number is cost per success,
   not hit rate. Nulls and losses are published.
6. **Advise and schedule; never proxy.** Maidan does not call models and does
   not cache model outputs. Semantic response caching "goes badly wrong on
   agentic traffic" in its own vendors' words, and a proxy is the gateway
   business, not the room's.
7. **Isolation over savings.** Nothing is shared across workspaces. A shared
   cache across tenants is a timing side channel.

## The program

Each item lists what it changes and what proves it. The Open Work entries carry
the full acceptance criteria.

### Phase 1: Maidan's own surfaces are cache-stable

- **C1. The context pack** (lane CTX1):
  - one canonical pack, the same bytes on REST and MCP, with deterministic ties;
  - layered in order: identity, the stable thread brief, glossary, parent grounding, accepted decisions, messages, the rest, then a volatile tail;
  - elision that folds in fixed blocks;
  - a split response with the prefix's sha256;
  - a workspace boot pack, which is the shared prefix;
  - delta packs since a cursor, so a long-running agent appends instead of re-reading;
  - exact `as_of`.
  
  Proof: a byte golden, REST/MCP parity, and a test that a new message leaves every byte before the messages layer unchanged.
- **C2. The MCP surface** (lane CTX2):
  - `server/discover`;
  - `ttlMs` and `cacheScope` on every cacheable result, chosen per result (content-addressed artifacts get the longest);
  - tool profiles as their own endpoints, a worker profile of about 14 tools and a reviewer profile, each list byte-identical for every client and sorted;
  - instructions whose first 250 characters carry what an agent must know, under 2,048 in all.
  
  Proof: a byte golden per profile, and a CI check that the MCP reference matches the catalog.
- **C3. A ledger that can price caching:**
  - `input` means uncached input on every provider, with the normalization table below;
  - cache writes split into 5-minute and 1-hour tiers;
  - each report carries the provider, the model the response named, the service tier, a batch flag, the harness and its version, the cache key or session, the provider's cache-miss reason when there is one, and the sha256 of each Maidan pack the call used;
  - a read API and an MCP tool for thread, member and workspace rollups (spend, hit rate, write share, dollars saved against the uncached price, and cost per completed task);
  - metrics, and the manager digest's spend line;
  - budgets that show each tier.
- **C4. The SDK and recipe layer:**
  - usage normalizers in the four SDKs (from Anthropic, Bedrock Converse, OpenAI Responses and Chat, Gemini, DeepSeek, Mistral, xAI and vLLM responses);
  - a boot-pack helper that places the shared prefix with a cache breakpoint;
  - recipes for Claude Code and the Agent SDK (`excludeDynamicSections`, fork over spawn), Codex, Goose, pi and OpenHands that say where Maidan's bytes land and how to keep them shared.

### Phase 2: coordinate for the cache

- **C5. Warm, then fan out.** When several claims share a prefix (recipe
  children, one channel's boot pack), Maidan releases one first and the rest
  once its first response has begun, signalled by its first usage report or a
  prefix-warm acknowledgement. Claim responses carry the prefix sha and its
  TTL left.

  By arithmetic, with prices as of 2026-10-01 (Sonnet 5.5, 40,000 shared tokens, 8 agents):

  | Approach | Cost |
  |---|---|
  | All 8 start cold in parallel | $0.80 |
  | No caching at all | $0.64 |
  | One writes, then seven read | $0.156 |
- **C6. Claim timing inside the TTL.** Affinity in `claim_next` for the agent
  whose cache is warm for that prefix. Keep-alive or 1-hour advice by the
  measured gap between an agent's calls.
- **C7. A batch lane.** A thread whose deadline has slack can be marked for
  batch or flex. Agents that use those tiers claim it, and the ledger shows the
  saving.
- **C8. Stop paying twice.** Each task gets a fingerprint, a hash of its brief
  and inputs. An identical open task is linked rather than run again, and an
  accepted result with identical inputs is offered for reuse. The ledger prices
  what was avoided.

### Phase 3: fleets and self-hosted inference

- **C9. Fleet profiles.** A published harness profile, the same build,
  capability set and API workspace, so a fleet shares one system and tools
  prefix. A conformance check reads the usage evidence.
- **C10. Summarize once.** Content-addressed thread digests served as MCP
  Skills or resources by sha256, written once and read by many.
- **C11. Self-hosted routing hints.** Session, parent and affinity headers
  (Dynamo, llm-d, AIBrix, the SGLang router, TensorRT-LLM) derived from thread
  and claim structure, emitted by the SDKs.
- **C12. Model and effort by task class,** learned from the ledger, never
  switched per turn (caches are model-scoped).

### Proof: MCEB-1

A pre-registered benchmark, protocol dated 2026-10-01:
- **Workspace and teams:** a seeded workspace from a signed export (its sha256 published), with tasks above and below the providers' cache minimums, and teams of 1, 3 and 8.
- **Arms:**
  - caching off;
  - provider best practice without coordination (published even if its hit rate is about 95%);
  - Maidan claims;
  - Maidan's cache-stable context;
  - the same bytes re-serialized;
  - 1-hour TTL with pre-warm;
  - self-hosted KV-aware routing.
- **Metrics:** cost per success first, then hit rate with the write tiers split, write share, duplicate attempts, abandoned spend and pass^k.
- **Statistics:** at least five repetitions, a task-clustered bootstrap, tokens published beside dollars so anyone can reprice, and billing reconciled.

Runs spend real money, so they wait for the maintainer's budget.

## Normalizing usage

| Provider (API) | Uncached input | Cache read | Cache write |
|---|---|---|---|
| Anthropic Messages | `input_tokens` | `cache_read_input_tokens` | `cache_creation_input_tokens`, split by `cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens` |
| Bedrock Converse | `inputTokens` | `cacheReadInputTokens` | `cacheWriteInputTokens` |
| OpenAI Responses, Azure | `input_tokens` minus read and write | `input_tokens_details.cached_tokens` | `input_tokens_details.cache_write_tokens` (GPT-5.6 and later) |
| OpenAI Chat, Azure, OpenRouter | `prompt_tokens` minus read and write | `prompt_tokens_details.cached_tokens` | `prompt_tokens_details.cache_write_tokens` |
| Gemini | `promptTokenCount` minus read | `cachedContentTokenCount` | none per request; explicit caches bill creation and storage |
| DeepSeek | `prompt_cache_miss_tokens` | `prompt_cache_hit_tokens` | none |
| Mistral, xAI, vLLM | `prompt_tokens` minus read | `prompt_tokens_details.cached_tokens` | none |

## What this is not

- Not a proxy, a gateway or a response cache.
- Not shared caching across workspaces or tenants.
- Not a hit-rate headline. The claim is cost per completed task, measured,
  dated and reproducible.

## The pitch

"Stop paying twice, and know what each finished task cost."

Until MCEB-1 has run, Maidan claims only what it can show: byte-stable context
with a golden test, a tools prefix of about 2,000 tokens instead of about
27,000 for a worker, and a ledger that prices every call's cache tiers.
