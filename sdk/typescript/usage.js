// Provider usage objects -> the ledger's `report_usage` shape.
//
// The ledger's `input` is uncached input on every provider, and cache writes
// are two tiers, 5-minute and 1-hour. Providers disagree on both, so each
// reader below says where its numbers come from. The Python, Go and Rust SDKs
// and the server's own reader agree on the fixtures in `sdk/usage-fixtures/`.

export class UsageError extends Error {
  constructor(message) {
    super(message);
    this.name = "UsageError";
  }
}

function at(obj, path) {
  let v = obj;
  for (const key of path.split(".")) {
    if (v === null || typeof v !== "object") return undefined;
    v = v[key];
  }
  return v;
}

function count(obj, path) {
  const v = at(obj, path);
  if (v === undefined || v === null) return 0;
  if (!Number.isSafeInteger(v) || v < 0) {
    throw new UsageError(`${path} must be a non-negative integer`);
  }
  return v;
}

function text(obj, path) {
  const v = at(obj, path);
  return typeof v === "string" && v.trim() !== "" ? v.trim() : undefined;
}

function add(...parts) {
  const total = parts.reduce((a, b) => a + b, 0);
  if (!Number.isSafeInteger(total)) throw new UsageError("token total overflow");
  return total;
}

// OpenAI, Gemini, Mistral, xAI and vLLM count cached tokens inside the total.
function uncached(total, ...cached) {
  const rest = total - add(...cached);
  if (rest < 0) throw new UsageError("input_tokens smaller than the cache tiers it includes");
  return rest;
}

function block(response, key) {
  const u = response[key];
  if (u === null || typeof u !== "object" || Array.isArray(u)) {
    throw new UsageError(`response has no ${key} object`);
  }
  return u;
}

// Bedrock Converse splits its writes by TTL in `cacheDetails`.
function bedrockDetails(u) {
  const details = u.cacheDetails;
  if (details === undefined || details === null) return [0, 0];
  if (!Array.isArray(details)) throw new UsageError("cacheDetails must be an array");
  let five = 0;
  let hour = 0;
  for (const d of details) {
    if (d === null || typeof d !== "object" || Array.isArray(d)) {
      throw new UsageError("cacheDetails entries must be objects");
    }
    const tokens = count(d, "inputTokens");
    if (d.ttl === "5m") five = add(five, tokens);
    else if (d.ttl === "1h") hour = add(hour, tokens);
    else throw new UsageError("cacheDetails ttl must be 5m or 1h");
  }
  return [five, hour];
}

function chatShape(u, { reasoningIsExtra = false } = {}) {
  const read = count(u, "prompt_tokens_details.cached_tokens");
  const write = count(u, "prompt_tokens_details.cache_write_tokens");
  const reasoning = reasoningIsExtra ? count(u, "completion_tokens_details.reasoning_tokens") : 0;
  return {
    input: uncached(count(u, "prompt_tokens"), read, write),
    output: add(count(u, "completion_tokens"), reasoning),
    cache_read: read,
    cache_write_5m: write,
    cache_write_1h: 0,
  };
}

function responsesShape(u, { reasoningIsExtra = false } = {}) {
  const read = count(u, "input_tokens_details.cached_tokens");
  const write = count(u, "input_tokens_details.cache_write_tokens");
  const reasoning = reasoningIsExtra ? count(u, "output_tokens_details.reasoning_tokens") : 0;
  return {
    input: uncached(count(u, "input_tokens"), read, write),
    output: add(count(u, "output_tokens"), reasoning),
    cache_read: read,
    cache_write_5m: write,
    cache_write_1h: 0,
  };
}

const READERS = {
  anthropic: {
    name: "anthropic",
    read(r) {
      const u = block(r, "usage");
      let five = count(u, "cache_creation.ephemeral_5m_input_tokens");
      const hour = count(u, "cache_creation.ephemeral_1h_input_tokens");
      if (five === 0 && hour === 0) five = count(u, "cache_creation_input_tokens");
      return {
        tokens: {
          input: count(u, "input_tokens"),
          output: count(u, "output_tokens"),
          cache_read: count(u, "cache_read_input_tokens"),
          cache_write_5m: five,
          cache_write_1h: hour,
        },
        model: text(r, "model"),
        serviceTier: text(u, "service_tier"),
        cacheMissReason: text(r, "diagnostics.cache_miss_reason.type"),
      };
    },
  },
  "bedrock-converse": {
    name: "aws.bedrock",
    read(r) {
      const u = block(r, "usage");
      let [five, hour] = bedrockDetails(u);
      if (five === 0 && hour === 0) five = count(u, "cacheWriteInputTokens");
      return {
        tokens: {
          input: count(u, "inputTokens"),
          output: count(u, "outputTokens"),
          cache_read: count(u, "cacheReadInputTokens"),
          cache_write_5m: five,
          cache_write_1h: hour,
        },
      };
    },
  },
  "openai-responses": {
    name: "openai",
    read(r) {
      return {
        tokens: responsesShape(block(r, "usage")),
        model: text(r, "model"),
        serviceTier: text(r, "service_tier"),
      };
    },
  },
  "openai-chat": {
    name: "openai",
    read(r) {
      return {
        tokens: chatShape(block(r, "usage")),
        model: text(r, "model"),
        serviceTier: text(r, "service_tier"),
      };
    },
  },
  gemini: {
    name: "gcp.gemini",
    read(r) {
      const u = block(r, "usageMetadata");
      const read = count(u, "cachedContentTokenCount");
      return {
        tokens: {
          input: uncached(count(u, "promptTokenCount"), read),
          output: add(count(u, "candidatesTokenCount"), count(u, "thoughtsTokenCount")),
          cache_read: read,
          cache_write_5m: 0,
          cache_write_1h: 0,
        },
        model: text(r, "modelVersion"),
        serviceTier: text(u, "serviceTier"),
      };
    },
  },
  deepseek: {
    name: "deepseek",
    read(r) {
      const u = block(r, "usage");
      if (u.prompt_cache_miss_tokens === undefined || u.prompt_cache_miss_tokens === null) {
        throw new UsageError("prompt_cache_miss_tokens is required");
      }
      return {
        tokens: {
          input: count(u, "prompt_cache_miss_tokens"),
          output: count(u, "completion_tokens"),
          cache_read: count(u, "prompt_cache_hit_tokens"),
          cache_write_5m: 0,
          cache_write_1h: 0,
        },
        model: text(r, "model"),
      };
    },
  },
  mistral: {
    name: "mistral_ai",
    read(r) {
      return { tokens: chatShape(block(r, "usage")), model: text(r, "model") };
    },
  },
  xai: {
    name: "x_ai",
    read(r) {
      const u = block(r, "usage");
      const tokens =
        u.input_tokens !== undefined
          ? responsesShape(u, { reasoningIsExtra: true })
          : chatShape(u, { reasoningIsExtra: true });
      return { tokens, model: text(r, "model") };
    },
  },
  vllm: {
    name: "vllm",
    read(r) {
      return { tokens: chatShape(block(r, "usage")), model: text(r, "model") };
    },
  },
};

/** The provider ids `normalizeUsage` accepts. */
export const USAGE_PROVIDERS = Object.freeze(Object.keys(READERS));

/**
 * Turn one provider response into `{ model, tokens, evidence }`, the
 * economic part of a `report_usage` body. `options.model` names the model when
 * the response does not (Bedrock Converse); `options.provider` overrides the
 * evidence provider name (a Chat Completions shape served by Azure).
 */
export function normalizeUsage(provider, response, options = {}) {
  const reader = READERS[provider];
  if (!reader) throw new UsageError(`unknown provider ${provider}`);
  if (response === null || typeof response !== "object" || Array.isArray(response)) {
    throw new UsageError("response must be a JSON object");
  }
  const got = reader.read(response);
  const model = (options.model ?? got.model ?? "").trim();
  if (!model) throw new UsageError("model is required: the response names none, so pass options.model");
  const evidence = { provider: options.provider ?? reader.name };
  if (got.serviceTier) evidence.service_tier = got.serviceTier;
  if (got.cacheMissReason) evidence.cache_miss_reason = got.cacheMissReason;
  return { model, tokens: got.tokens, evidence };
}

const TIERS = [
  ["input", "input_usd_micros_per_million"],
  ["output", "output_usd_micros_per_million"],
  ["cache_read", "cache_read_usd_micros_per_million"],
  ["cache_write_5m", "cache_write_5m_usd_micros_per_million"],
  ["cache_write_1h", "cache_write_1h_usd_micros_per_million"],
];

/**
 * `usd_micros` as the ledger checks it: the sum of tokens times micro-USD per
 * million, divided by a million and rounded up. Integer arithmetic, so a large
 * report is exact.
 */
export function usdMicros(tokens, priceSnapshot) {
  let sum = 0n;
  for (const [tier, rate] of TIERS) {
    const n = tokens[tier] ?? 0;
    const r = priceSnapshot[rate] ?? 0;
    if (!Number.isSafeInteger(n) || n < 0) throw new UsageError(`${tier} must be a non-negative integer`);
    if (!Number.isSafeInteger(r) || r < 0) throw new UsageError(`${rate} must be a non-negative integer`);
    sum += BigInt(n) * BigInt(r);
  }
  const usd = (sum + 999_999n) / 1_000_000n;
  if (usd > BigInt(Number.MAX_SAFE_INTEGER)) throw new UsageError("usd_micros overflow");
  return Number(usd);
}
