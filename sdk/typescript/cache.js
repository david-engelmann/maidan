// Keeping Maidan's bytes in the provider's cache: the boot prefix with a cache
// breakpoint, one cache key per shared-prefix group, and the thread id as a
// gateway session id. Pure functions; the Python, Go and Rust SDKs give the
// same output for the same input (docs/Harness Caching.md).

export class CacheError extends Error {
  constructor(message) {
    super(message);
    this.name = "CacheError";
  }
}

async function sha256Hex(text) {
  const bytes = new TextEncoder().encode(text);
  const subtle = globalThis.crypto?.subtle ?? (await import("node:crypto")).webcrypto.subtle;
  const digest = new Uint8Array(await subtle.digest("SHA-256", bytes));
  return Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** The boot bytes as served, and their sha256 for `evidence.pack_sha256`. */
export async function bootPrefix(bytes) {
  const text = typeof bytes === "string" ? bytes : new TextDecoder().decode(bytes);
  return { text, sha256: await sha256Hex(text) };
}

const TTLS = new Set(["5m", "1h"]);

/**
 * Place `text` (the boot prefix) as the first, cached part of a request.
 * Anthropic and Bedrock get an explicit breakpoint and take `ttl` ("5m" or
 * "1h"); OpenAI Responses gets an explicit breakpoint (GPT-5.6 and later, 30
 * minutes); the rest cache a matching prefix on their own, so the prefix is
 * only put first.
 */
export function cachedPrefix(provider, text, options = {}) {
  const { ttl } = options;
  if (ttl !== undefined && !TTLS.has(ttl)) throw new CacheError("ttl must be 5m or 1h");
  if (ttl !== undefined && provider !== "anthropic" && provider !== "bedrock-converse") {
    throw new CacheError(`${provider} takes no cache ttl`);
  }
  const longTtl = ttl === "1h" ? { ttl: "1h" } : {};
  switch (provider) {
    case "anthropic":
      return { type: "text", text, cache_control: { type: "ephemeral", ...longTtl } };
    case "bedrock-converse":
      return [{ text }, { cachePoint: { type: "default", ...longTtl } }];
    case "openai-responses":
      return {
        type: "message",
        role: "developer",
        content: [{ type: "input_text", text, prompt_cache_breakpoint: { mode: "explicit" } }],
      };
    case "gemini":
      return { parts: [{ text }] };
    case "openai-chat":
    case "deepseek":
    case "mistral":
    case "xai":
    case "vllm":
      return { role: "system", content: text };
    default:
      throw new CacheError(`unknown provider ${provider}`);
  }
}

/**
 * One cache key per shared-prefix group, never shared across workspaces: the
 * workspace id is hashed in, so the same group name in two workspaces gives
 * two keys, and the key reveals neither.
 */
export async function cacheKey(workspaceId, group) {
  if (!workspaceId || !group) throw new CacheError("workspace id and group are required");
  return `maidan-${(await sha256Hex(`${workspaceId}\n${group}`)).slice(0, 32)}`;
}

/** Where `key` goes for a provider that takes one; `{}` for one that does not. */
export function cacheKeyFields(provider, key) {
  switch (provider) {
    case "openai-responses":
    case "openai-chat":
    case "mistral":
    case "xai-responses":
      return { body: { prompt_cache_key: key } };
    case "xai-chat":
      return { headers: { "x-grok-conv-id": key } };
    case "deepseek":
      return { body: { user_id: key } };
    case "deepseek-anthropic":
      return { body: { metadata: { user_id: key } } };
    case "vllm":
      return { body: { cache_salt: key } };
    case "anthropic":
    case "bedrock-converse":
    case "gemini":
      return {};
    default:
      throw new CacheError(`unknown provider ${provider}`);
  }
}

const UUID_V7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

/**
 * The thread id as the gateway's session id, so the gateway's spend joins the
 * thread's outcome. Helicone takes an optional `path` and `name`.
 */
export function gatewaySession(gateway, threadId, options = {}) {
  if (!threadId) throw new CacheError("thread id is required");
  switch (gateway) {
    case "openrouter":
      return { body: { session_id: threadId } };
    case "helicone":
      return {
        headers: {
          "Helicone-Session-Id": threadId,
          "Helicone-Session-Path": options.path ?? "/",
          "Helicone-Session-Name": options.name ?? "maidan",
        },
      };
    case "litellm":
      return { body: { litellm_session_id: threadId } };
    case "tensorzero":
    case "tensorzero-native":
      if (!UUID_V7.test(threadId)) {
        throw new CacheError("TensorZero takes a UUIDv7 episode id; this thread id is not one");
      }
      return gateway === "tensorzero"
        ? { body: { "tensorzero::episode_id": threadId } }
        : { body: { episode_id: threadId } };
    default:
      throw new CacheError(`unknown gateway ${gateway}`);
  }
}
