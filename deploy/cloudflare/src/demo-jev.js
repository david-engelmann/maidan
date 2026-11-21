// The demo's way to a decision model, for advice on a hand-off
// (docs/Decision-Models.md). The demo container has no internet. Its server
// asks an `.internal` name for each provider, which the Worker intercepts
// and forwards from here with the Worker's own key for that provider, so the
// container never holds a credential and can reach nothing else.
//
// Kept apart from demo.js so the tests run it without the Containers runtime.

// What the container's key reads. It is not one: the Worker swaps in its own
// on the way out.
export const JEV_PLACEHOLDER_KEY = "held-by-the-worker";

// Each provider the demo can ask: the name the demo's server calls (which
// `outboundByHost` routes here), where its calls go, the Worker secret that
// holds its key, the two calls the server makes (the questions, and the key
// check), how the key is sent, and what the server is started with.
export const RELAYS = [
  {
    id: "typesafe",
    name: "TypeSafe Jev",
    host: "jev.internal",
    origin: "https://api.typesafe.ai",
    keyVar: "TYPESAFE_API_KEY",
    ask: "/v1/systemone",
    models: "/v1/models",
    auth: (key) => ({ authorization: `Bearer ${key}` }),
    baseUrlVar: "MAIDAN_JEV_BASE_URL",
  },
  {
    id: "openai",
    name: "OpenAI Decisions",
    host: "openai-decisions.internal",
    origin: "https://api.openai.com",
    keyVar: "OPENAI_API_KEY",
    ask: "/v1/decisions",
    models: "/v1/models",
    auth: (key) => ({ authorization: `Bearer ${key}` }),
    baseUrlVar: "MAIDAN_OPENAI_DECISIONS_BASE_URL",
  },
  {
    id: "google",
    name: "Gemini Decisions",
    host: "gemini-decisions.internal",
    origin: "https://generativelanguage.googleapis.com",
    keyVar: "GEMINI_API_KEY",
    ask: "/v1beta/decisions",
    models: "/v1beta/models",
    auth: (key) => ({ "x-goog-api-key": key }),
    baseUrlVar: "MAIDAN_GEMINI_DECISIONS_BASE_URL",
  },
  {
    id: "perplexity",
    name: "Perplexity Decider",
    host: "perplexity-decisions.internal",
    origin: "https://api.perplexity.ai",
    keyVar: "PERPLEXITY_API_KEY",
    ask: "/v1/decisions",
    models: "/v1/models",
    auth: (key) => ({ authorization: `Bearer ${key}` }),
    baseUrlVar: "MAIDAN_PERPLEXITY_DECISIONS_BASE_URL",
  },
  // Cloudflare's Clef is a provider the server offers, and not one the demo
  // asks: its key is an account's token, and the demo holds none for it.
];

const TYPESAFE = RELAYS[0];
// The first provider's own names, as the tests and the wrangler notes use them.
export const JEV_INTERNAL_HOST = TYPESAFE.host;
export const JEV_ORIGIN = TYPESAFE.origin;
export const JEV_KEY_VAR = TYPESAFE.keyVar;
// A rate limit binding (wrangler.demo.jsonc) on forwarded questions, so the
// demo's visitors together cannot run past a small, known bill. One limit
// covers every provider.
export const JEV_LIMIT_BINDING = "JEV_LIMIT";
// The largest question the server sends: its own cap on a request to a
// provider (MAX_REQUEST_BYTES in crates/maidan-server/src/land_gate_advisor.rs,
// which test/demo-jev.test.js holds this to). With the rate limit it sets
// about what a day of the demo can cost; the limiter is approximate.
export const JEV_MAX_BODY_BYTES = 16 * 1024;

const held = (env, relay) => {
  const key = env?.[relay.keyVar];
  return typeof key === "string" && key.trim() !== "" ? key.trim() : null;
};

/** The providers the Worker holds a key for, in the order the server tries them. */
export function adviceProviders(env) {
  return RELAYS.filter((relay) => held(env, relay)).map(({ id, name }) => ({ id, name }));
}

/** Whether the demo offers advice at all: a key for at least one provider. */
export function jevEnabled(env) {
  return adviceProviders(env).length > 0;
}

// What the demo's server is started with: advice on, each provider the Worker
// holds a key for pointed at its internal name, and those names (and no
// others) allowed through the egress guard.
export function adviceServerEnv(env) {
  const on = RELAYS.filter((relay) => held(env, relay));
  if (on.length === 0) return {};
  const vars = {
    MAIDAN_JEV_LAND_GATE_ENABLED: "1",
    MAIDAN_INTERNAL_EGRESS_HOSTS: on.map((relay) => relay.host).join(","),
  };
  for (const relay of on) {
    vars[relay.baseUrlVar] = `http://${relay.host}/`;
    vars[relay.keyVar] = JEV_PLACEHOLDER_KEY;
  }
  return vars;
}

// What the server was started with when TypeSafe's was the only key.
export const JEV_SERVER_ENV = adviceServerEnv({ [TYPESAFE.keyVar]: "set" });

function refuse(status, detail) {
  return new Response(JSON.stringify({ detail }), { status, headers: { "content-type": "application/json" } });
}

// The request's body, or `null` once it runs past `max` bytes: the read stops
// there and the rest is never buffered, whatever Content-Length said.
async function readCapped(request, max) {
  if (!request.body) return new Uint8Array(0);
  const reader = request.body.getReader();
  const chunks = [];
  let total = 0;
  let next = await reader.read();
  while (!next.done) {
    total += next.value.byteLength;
    if (total > max) {
      await reader.cancel();
      return null;
    }
    chunks.push(next.value);
    next = await reader.read();
  }
  const body = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) {
    body.set(chunk, at);
    at += chunk.byteLength;
  }
  return body;
}

// Forward one of the server's calls to a provider with the Worker's key for
// it. Any other path or method is refused here, a body over the cap is
// refused without being read past it, and a question over the rate limit is
// a 429 the server reports as "the model did not answer".
export async function forwardAdvice(relay, request, env, fetchFn = fetch) {
  const key = held(env, relay);
  if (!key) return refuse(503, "advice is not configured for the demo");
  const url = new URL(request.url);
  const asking = request.method === "POST" && url.pathname === relay.ask;
  const checking = request.method === "GET" && url.pathname === relay.models;
  if (!asking && !checking) return refuse(404, "no such path");
  const init = { method: request.method, headers: { ...relay.auth(key), accept: "application/json" } };
  if (asking) {
    const declared = Number(request.headers.get("content-length") ?? "0");
    if (Number.isFinite(declared) && declared > JEV_MAX_BODY_BYTES) return refuse(413, "the hand-off is too large for the demo");
    const body = await readCapped(request, JEV_MAX_BODY_BYTES);
    if (body === null) return refuse(413, "the hand-off is too large for the demo");
    const limiter = env?.[JEV_LIMIT_BINDING];
    if (limiter) {
      const { success } = await limiter.limit({ key: "demo" });
      if (!success) return refuse(429, "the demo is asking for advice too fast; try again in a minute");
    }
    init.headers["content-type"] = "application/json";
    init.body = body;
  }
  return fetchFn(`${relay.origin}${url.pathname}`, init);
}

/** TypeSafe's relay, by its old name. */
export function forwardJev(request, env, fetchFn = fetch) {
  return forwardAdvice(TYPESAFE, request, env, fetchFn);
}

// The `outboundByHost` map for the demo's container: one name per provider.
export const jevOutboundByHost = Object.fromEntries(
  RELAYS.map((relay) => [relay.host, (request, env) => forwardAdvice(relay, request, env)]),
);
