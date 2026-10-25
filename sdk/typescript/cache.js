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

const ONE_KEY_ROUTERS = {
  dynamo: { header: "X-Dynamo-Session-ID", max: Infinity },
  "llm-d": { header: "x-session-id", max: Infinity },
  aibrix: { header: "x-aibrix-session-key", max: 256 },
  sglang: { header: "X-SMG-Routing-Key", max: 128 },
};

/**
 * Affinity headers for a self-hosted KV-aware router, from thread and claim
 * structure. Every router keys a child by its parent, so a child lands where its parent's
 * prefix is warm: Dynamo, llm-d, AIBrix and SGLang all get the parent's thread
 * as their one affinity key when there is one. Dynamo's docs say affinity uses
 * `X-Dynamo-Session-ID` alone (with `--router-session-affinity-ttl-secs` set)
 * and `X-Dynamo-Parent-Session-ID` is trace metadata, so a child sends its
 * parent's thread as the session and no parent header (its own id is not a
 * parent). AIBrix also gets `routing-strategy: session-affinity`. SGLang caps
 * the key at 128 bytes and AIBrix at 256; a longer key is an error. Hints only:
 * no router accepts a prefix hash from a client.
 */
export function routingHints(router, threadId, options = {}) {
  if (!threadId) throw new CacheError("thread id is required");
  const parent = options.parentThreadId || undefined;
  const spec = Object.hasOwn(ONE_KEY_ROUTERS, router) ? ONE_KEY_ROUTERS[router] : undefined;
  if (!spec) throw new CacheError(`unknown router ${router}`);
  const key = parent ?? threadId;
  if (new TextEncoder().encode(key).length > spec.max) {
    throw new CacheError(`routing key is longer than ${spec.max} bytes`);
  }
  const headers = { [spec.header]: key };
  if (router === "aibrix") headers["routing-strategy"] = "session-affinity";
  return { headers };
}

/** The shortest salt key `cacheSalt` accepts, in bytes. */
export const MIN_SALT_KEY_BYTES = 32;

/**
 * vLLM's `cache_salt` for one workspace: HMAC-SHA256(saltKey,
 * "cache_salt\n" + workspaceId) as unpadded base64url (43 characters), so it
 * cannot be computed from the workspace id alone. The key must be at least 32
 * bytes. Set the salt from a trusted server-side component that holds the key,
 * never from an end client. It is a cache partition, not an auth token.
 */
export async function cacheSalt(workspaceId, saltKey) {
  if (!workspaceId) throw new CacheError("workspace id is required");
  const key = typeof saltKey === "string" ? new TextEncoder().encode(saltKey) : saltKey;
  if (!(key instanceof Uint8Array) || key.length < MIN_SALT_KEY_BYTES) {
    throw new CacheError(`salt key must be at least ${MIN_SALT_KEY_BYTES} bytes`);
  }
  const subtle = globalThis.crypto?.subtle ?? (await import("node:crypto")).webcrypto.subtle;
  const k = await subtle.importKey("raw", key, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const mac = new Uint8Array(
    await subtle.sign("HMAC", k, new TextEncoder().encode(`cache_salt\n${workspaceId}`)),
  );
  const b64 = btoa(String.fromCharCode(...mac));
  return { body: { cache_salt: b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "") } };
}
