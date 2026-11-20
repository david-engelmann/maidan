// The demo's way to TypeSafe's Jev, for advice on a hand-off
// (docs/Decision-Models.md). The demo container has no internet. Its server
// asks `http://jev.internal/`, which the Worker intercepts and forwards from
// here with the Worker's own key, so the container never holds a credential
// and can reach nothing else.
//
// Kept apart from demo.js so the tests run it without the Containers runtime.

// The name the demo's server calls. `outboundByHost` routes it here.
export const JEV_INTERNAL_HOST = "jev.internal";
// Where the calls go.
export const JEV_ORIGIN = "https://api.typesafe.ai";
// The Worker secret that holds the key. Unset, advice is off in the demo.
export const JEV_KEY_VAR = "TYPESAFE_API_KEY";
// A rate limit binding (wrangler.demo.jsonc) on forwarded questions, so the
// demo's visitors together cannot run past a small, known bill.
export const JEV_LIMIT_BINDING = "JEV_LIMIT";
// A hand-off is a few kilobytes. The demo's own body cap is the same.
export const JEV_MAX_BODY_BYTES = 64 * 1024;

// What the container's key reads. It is not one: the Worker swaps in its own
// on the way out.
export const JEV_PLACEHOLDER_KEY = "held-by-the-worker";

export function jevEnabled(env) {
  const key = env?.[JEV_KEY_VAR];
  return typeof key === "string" && key.trim() !== "";
}

// What the demo's server is started with when the Worker holds a key: advice
// on, pointed at the internal name, which the egress guard is told to allow.
export const JEV_SERVER_ENV = {
  MAIDAN_JEV_LAND_GATE_ENABLED: "1",
  MAIDAN_JEV_BASE_URL: `http://${JEV_INTERNAL_HOST}/`,
  TYPESAFE_API_KEY: JEV_PLACEHOLDER_KEY,
  MAIDAN_INTERNAL_EGRESS_HOSTS: JEV_INTERNAL_HOST,
};

function refuse(status, detail) {
  return new Response(JSON.stringify({ detail }), { status, headers: { "content-type": "application/json" } });
}

// The two calls the server makes: the questions, and the key check.
function allowed(method, pathname) {
  return (method === "POST" && pathname === "/v1/systemone") || (method === "GET" && pathname === "/v1/models");
}

// Forward one of the server's calls to TypeSafe with the Worker's key. Any
// other path or method is refused here, a body over the cap is refused before
// it is read past the cap, and a question over the rate limit is a 429 the
// server reports as "the model did not answer".
export async function forwardJev(request, env, fetchFn = fetch) {
  const key = env?.[JEV_KEY_VAR];
  if (!jevEnabled(env)) return refuse(503, "advice is not configured for the demo");
  const url = new URL(request.url);
  if (!allowed(request.method, url.pathname)) return refuse(404, "no such path");
  const init = {
    method: request.method,
    headers: { authorization: `Bearer ${key.trim()}`, accept: "application/json" },
  };
  if (request.method === "POST") {
    const declared = Number(request.headers.get("content-length") ?? "0");
    if (Number.isFinite(declared) && declared > JEV_MAX_BODY_BYTES) return refuse(413, "the hand-off is too large for the demo");
    const body = await request.arrayBuffer();
    if (body.byteLength > JEV_MAX_BODY_BYTES) return refuse(413, "the hand-off is too large for the demo");
    const limiter = env?.[JEV_LIMIT_BINDING];
    if (limiter) {
      const { success } = await limiter.limit({ key: "demo" });
      if (!success) return refuse(429, "the demo is asking for advice too fast; try again in a minute");
    }
    init.headers["content-type"] = "application/json";
    init.body = body;
  }
  return fetchFn(`${JEV_ORIGIN}${url.pathname}`, init);
}

// The `outboundByHost` map for the demo's container.
export const jevOutboundByHost = {
  [JEV_INTERNAL_HOST]: (request, env) => forwardJev(request, env),
};
