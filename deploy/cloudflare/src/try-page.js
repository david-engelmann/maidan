// What try.maidan.world's Worker answers itself, before the container: the
// "try" page and its files, which never wake the container, and the
// Turnstile check on `POST /sandbox`, which the container cannot make (it
// has no internet). Kept apart from demo.js so the tests run it without the
// Containers runtime.

// The page and its files, served from the Worker's static assets
// (deploy/cloudflare/try, the `ASSETS` binding in wrangler.demo.jsonc).
// /theme.js is the console's pre-paint script (web/console/public/theme.js),
// which deploy-demo.yml copies in: the console's /ui/theme.js is the
// container's, and loading it would wake the container on every visit.
export const TRY_PATHS = new Set(["/", "/theme.js", "/try.css", "/try.js", "/clients.js", "/geist.woff2"]);

// What the page reads to know whether to render the Turnstile widget.
export const TRY_CONFIG_PATH = "/try-config.json";

// The Worker's own settings, never passed to the container: the widget's
// public key (a var) and its secret (a Worker secret). Both unset, the page
// shows no widget and the mint takes no token.
export const TURNSTILE_SITE_KEY_VAR = "DEMO_TURNSTILE_SITE_KEY";
export const TURNSTILE_SECRET_VAR = "DEMO_TURNSTILE_SECRET";

export const TURNSTILE_ORIGIN = "https://challenges.cloudflare.com";
export const SITEVERIFY_URL = `${TURNSTILE_ORIGIN}/turnstile/v0/siteverify`;
// The action the widget renders with and the check requires back.
export const TURNSTILE_ACTION = "sandbox";

// The page runs only its own script, posts only to its own origin, and loads
// Cloudflare's challenge only when the widget is on.
export function tryCsp(withTurnstile) {
  const challenge = withTurnstile ? ` ${TURNSTILE_ORIGIN}` : "";
  return [
    "default-src 'none'",
    `script-src 'self'${challenge}`,
    "style-src 'self'",
    "font-src 'self'",
    "img-src 'self'",
    "connect-src 'self'",
    `frame-src${withTurnstile ? challenge : " 'none'"}`,
    "base-uri 'none'",
    "form-action 'none'",
    "frame-ancestors 'none'",
  ].join("; ");
}

function problem(status, detail) {
  return new Response(JSON.stringify({ type: "about:blank", status, detail }), {
    status,
    headers: { "content-type": "application/problem+json", "cache-control": "no-store" },
  });
}

function turnstileOn(env) {
  return Boolean(env[TURNSTILE_SECRET_VAR] && env[TURNSTILE_SITE_KEY_VAR]);
}

// Check a mint's Turnstile token with Cloudflare. `null` when it passes (or
// the check is off); otherwise the refusal to send back. Fails closed.
export async function checkTurnstile(request, env, fetchImpl = fetch) {
  if (!turnstileOn(env)) return null;
  let token = "";
  try {
    const body = await request.clone().json();
    token = typeof body?.turnstile_token === "string" ? body.turnstile_token.trim() : "";
  } catch {
    token = "";
  }
  if (!token || token.length > 2048) {
    return problem(403, "complete the human check before starting a sandbox");
  }
  const form = new FormData();
  form.append("secret", env[TURNSTILE_SECRET_VAR]);
  form.append("response", token);
  const ip = request.headers.get("cf-connecting-ip");
  if (ip) form.append("remoteip", ip);
  let answer;
  try {
    const res = await fetchImpl(SITEVERIFY_URL, { method: "POST", body: form });
    if (!res.ok) return problem(502, "the human check could not be verified right now; try again");
    answer = await res.json();
  } catch {
    return problem(502, "the human check could not be verified right now; try again");
  }
  const host = new URL(request.url).hostname;
  if (answer?.success === true && answer.hostname === host && answer.action === TURNSTILE_ACTION) {
    return null;
  }
  return problem(403, "the human check was not accepted; solve it again");
}

// The Worker's own answer to `request`, or `null` to send it to the
// container.
export async function handleTry(request, env, fetchImpl = fetch) {
  const url = new URL(request.url);
  const read = request.method === "GET" || request.method === "HEAD";
  if (read && url.pathname === TRY_CONFIG_PATH) {
    const siteKey = turnstileOn(env) ? env[TURNSTILE_SITE_KEY_VAR] : null;
    return new Response(JSON.stringify({ turnstile_site_key: siteKey }), {
      headers: { "content-type": "application/json", "cache-control": "no-store" },
    });
  }
  if (read && TRY_PATHS.has(url.pathname)) {
    const asset = await env.ASSETS.fetch(new Request(url.toString(), { method: request.method }));
    const out = new Response(asset.body, asset);
    if (url.pathname === "/") {
      out.headers.set("content-security-policy", tryCsp(turnstileOn(env)));
      out.headers.set("cache-control", "no-store");
    }
    return out;
  }
  if (request.method === "POST" && url.pathname === "/sandbox") {
    return checkTurnstile(request, env, fetchImpl);
  }
  return null;
}
