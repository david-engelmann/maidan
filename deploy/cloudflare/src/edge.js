// What the Worker decides before and after the container: plain HTTP goes to
// HTTPS, a browser opening the bare origin of an instance without a site
// lands in the console, and every answer carries the headers that keep a browser on HTTPS and off content
// sniffing. Kept apart from the Worker so it is testable without the Workers
// runtime.

// Internal hosts: names under `.internal` that the container calls over plain
// HTTP and one of our own Workers answers through a service binding, so a
// webhook to another Worker never crosses the public internet and that Worker
// needs no public API. `MAIDAN_INTERNAL_HOSTS` maps each name to a binding
// (`hooks.internal=HOOKS,worker.internal=WORKER`).
// A name not under `.internal`, or one whose binding this deploy does not
// declare, is left out: every other host keeps the container's own internet
// egress, which the server needs for Neon, R2, GitHub and Slack.
const INTERNAL_HOST = /^(?:[a-z0-9-]+\.)+internal$/;
const BINDING_NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

// The `[host, binding]` pairs of `raw` that this deploy can serve, the first
// mention of a host winning.
export function parseInternalHosts(raw, env = {}) {
  const pairs = new Map();
  for (const entry of String(raw ?? "").split(",")) {
    const [rawHost, rawBinding, ...rest] = entry.split("=");
    if (rawBinding === undefined || rest.length > 0) continue;
    const host = rawHost.trim().toLowerCase().replace(/\.+$/, "");
    const binding = rawBinding.trim();
    if (!INTERNAL_HOST.test(host) || !BINDING_NAME.test(binding) || pairs.has(host)) continue;
    if (typeof env[binding]?.fetch !== "function") continue;
    pairs.set(host, binding);
  }
  return [...pairs];
}

// The server's `MAIDAN_INTERNAL_EGRESS_HOSTS`: the names in `rawEgress` that
// `rawMap` also routes to a declared binding, or undefined when none is. A
// listed name the Worker does not route would otherwise be accepted as a
// webhook target and then fail at every delivery.
export function routedEgressHosts(rawEgress, rawMap, env = {}) {
  const routed = new Set(parseInternalHosts(rawMap, env).map(([host]) => host));
  const hosts = String(rawEgress ?? "")
    .split(",")
    .map((entry) => entry.trim().toLowerCase().replace(/\.+$/, ""))
    .filter((host, i, all) => routed.has(host) && all.indexOf(host) === i);
  return hosts.length > 0 ? hosts.join(",") : undefined;
}

// The `outboundByHost` handlers for `raw`: each forwards the container's
// request, unchanged, to its binding. The binding is read from the `env` the
// handler is called with, the Worker's own.
export function internalOutboundByHost(raw, env = {}) {
  return Object.fromEntries(
    parseInternalHosts(raw, env).map(([host, binding]) => [
      host,
      (request, handlerEnv) => handlerEnv[binding].fetch(request),
    ]),
  );
}

// A year of HSTS for the zone's hosts, all of which serve HTTPS only.
export const SECURITY_HEADERS = {
  "strict-transport-security": "max-age=31536000; includeSubDomains",
  "x-content-type-options": "nosniff",
  "referrer-policy": "strict-origin-when-cross-origin",
};

// The hosted site's pages (privacy, terms, support, and the API's problem
// types), served from the Worker's static assets where an instance has them.
// Nothing else in the container lives at these paths. The front page is the
// server's own waitlist page, which links the first two
// (`MAIDAN_SIGNUP_PRIVACY_URL`, `_TERMS_URL`).
export const SITE_PATHS = new Set(["/privacy", "/terms", "/support", "/problems", "/site.css"]);

// Every problem `type` the server sends is `https://maidan.world/problems/`
// plus one of these, and each has a row on the problems page.
// `test/problems.test.js` holds this list to the server's.
export const PROBLEM_TYPES = new Set([
  "bad-request",
  "unauthorized",
  "invalid-signature",
  "forbidden",
  "not-found",
  "method-not-allowed",
  "conflict",
  "idempotency-key-in-flight",
  "cursor-too-old",
  "event-log-broken",
  "media-quota-exceeded",
  "footprint-conflict",
  "plan-conflict",
  "claim-lost",
  "thread-ended",
  "gone",
  "payload-too-large",
  "unsupported-media-type",
  "idempotency-key-reused",
  "unprocessable-entity",
  "rate-limited",
  "internal",
  "bad-gateway",
  "overloaded",
]);

// The site page a browser is shown when the container has no page at a path.
export const NOT_FOUND_PAGE = "/not-found";

// The site page a browser is shown while the container is starting or being
// replaced by a deploy, and how long either kind of client is told to wait.
export const UNAVAILABLE_PAGE = "/unavailable";
export const UNAVAILABLE_RETRY_SECS = 20;

// What the Containers runtime (`@cloudflare/containers`) answers, as plain
// text, when it has no running server to hand a request to: during a deploy's
// replacement, an operator restart, or a start that failed. The server itself
// never answers in plain text. Each prefix says whether the request can have
// reached a server: "unreached" when the runtime never handed it over (no
// instance, a start that failed, nothing listening), "unknown" otherwise.
const NO_CONTAINER = [
  ["Error proxying request to container", "unknown"],
  ["Failed to start container", "unreached"],
  ["There is no Container instance available", "unreached"],
  ["Container suddenly disconnected", "unknown"],
];

// The proxy error for a container with no listener: the request was never
// handed over.
const NOT_LISTENING = /not listening|not running|connection refused/i;

// Whether the container's answer means there was no server to answer: null
// for a real answer, else "unreached" or "unknown" (see NO_CONTAINER).
export async function containerUnavailableKind(response) {
  if (response.status < 500 || response.status === 501) {
    return null;
  }
  if (!(response.headers.get("content-type") ?? "").startsWith("text/plain")) {
    return null;
  }
  const text = await response.clone().text();
  const hit = NO_CONTAINER.find(([prefix]) => text.startsWith(prefix));
  if (!hit) {
    return null;
  }
  return hit[1] === "unknown" && NOT_LISTENING.test(text) ? "unreached" : hit[1];
}

// Whether the container's answer means there was no server to answer.
export async function isContainerUnavailable(response) {
  return (await containerUnavailableKind(response)) !== null;
}

// The answer for an API client while there is no container: the server's own
// `overloaded` problem, which says a retry is safe, with Retry-After.
export function unavailableProblem() {
  return new Response(
    JSON.stringify({
      type: "https://maidan.world/problems/overloaded",
      title: "Service Unavailable",
      status: 503,
      detail: `The server is starting or being updated. Retry in ${UNAVAILABLE_RETRY_SECS} seconds.`,
    }),
    {
      status: 503,
      headers: {
        "content-type": "application/problem+json",
        "retry-after": String(UNAVAILABLE_RETRY_SECS),
        "cache-control": "no-store",
      },
    },
  );
}

// The hosted site's own pages, linked from the waitlist page's footer next to
// the instance's privacy policy and terms.
export const HOSTED_FOOTER_LINKS = [
  ["/support", "Support"],
  ["/status", "Status"],
];

// The site's pages run no script and load only their own stylesheet.
export const SITE_CSP =
  "default-src 'none'; style-src 'self'; img-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

// Whether a request is for one of the site's pages: a GET or HEAD at a site
// path.
export function isSitePage(request) {
  if (request.method !== "GET" && request.method !== "HEAD") {
    return false;
  }
  return SITE_PATHS.has(new URL(request.url).pathname);
}

// Whether a request is a browser opening a page, rather than a script or an
// API client asking for data.
export function isPageNavigation(request) {
  if (request.method !== "GET" && request.method !== "HEAD") {
    return false;
  }
  const dest = request.headers.get("sec-fetch-dest");
  if (dest !== null) {
    return dest === "document";
  }
  return (request.headers.get("accept") ?? "").includes("text/html");
}

// The redirect a request gets before it reaches the container, or null. An
// instance with a site lets the server's waitlist page answer `/` instead of
// sending it to the console, and sends a problem type's URI to its row on the
// problems page.
export function edgeRedirect(request, { hasSite = false } = {}) {
  const url = new URL(request.url);
  if (url.protocol === "http:") {
    url.protocol = "https:";
    return Response.redirect(url.toString(), 301);
  }
  const problem = url.pathname.match(/^\/problems\/([a-z-]+)$/);
  const read = request.method === "GET" || request.method === "HEAD";
  if (hasSite && read && problem && PROBLEM_TYPES.has(problem[1])) {
    return Response.redirect(new URL(`/problems#${problem[1]}`, url).toString(), 302);
  }
  // The server answers `/` for API clients; only a browser asking for a page
  // is sent to the console.
  const accept = request.headers.get("accept") ?? "";
  if (!hasSite && url.pathname === "/" && request.method === "GET" && accept.includes("text/html")) {
    return Response.redirect(new URL("/ui/", url).toString(), 302);
  }
  return null;
}

// The container's answer with the security headers added where it set none.
// A WebSocket upgrade is passed through as it is: rebuilding it would drop
// the socket.
export function withSecurityHeaders(response, extra = {}) {
  if (response.status === 101 || response.webSocket) {
    return response;
  }
  const out = new Response(response.body, response);
  for (const [name, value] of Object.entries(extra)) {
    out.headers.set(name, value);
  }
  for (const [name, value] of Object.entries(SECURITY_HEADERS)) {
    if (!out.headers.has(name)) {
      out.headers.set(name, value);
    }
  }
  return out;
}

const escapeHtml = (text) =>
  String(text).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

// Whether a person's browser was handed one of the server's problem bodies:
// a page navigation answered with application/problem+json. A sign-up link
// that expired, or an identity the invite was not for, ends this way.
export function isProblemForAPerson(request, response) {
  return (
    isPageNavigation(request) &&
    response.status >= 400 &&
    (response.headers.get("content-type") ?? "").startsWith("application/problem+json")
  );
}

// The page a person sees in place of a problem body: its title and detail in
// plain words, the request id to quote, and where to go next. The detail is the
// server's own client-facing text, which never carries an internal cause.
export function problemPage(problem, requestId) {
  const title = typeof problem?.title === "string" && problem.title ? problem.title : "Something went wrong";
  const detail = typeof problem?.detail === "string" && problem.detail ? problem.detail : "";
  const ref = requestId ? `<p class="small">If you write to support, include this reference: <code>${escapeHtml(requestId)}</code></p>` : "";
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>${escapeHtml(title)} · Maidan</title>
<meta name="color-scheme" content="light">
<meta name="theme-color" content="#f2eee6">
<link rel="stylesheet" href="/site.css">
</head>
<body>
<a class="skip" href="#content">Skip to content</a>
<header class="top">
  <a class="brand" href="/"><svg class="mark" viewBox="0 0 64 64" aria-hidden="true"><path d="M38.95,32.85L48.07,37.53L59.65,35.70L59.74,34.95L48.69,28.76Z" fill="currentColor"/><path d="M36.31,37.52L39.45,47.28L46.40,51.08L47.18,50.46L46.09,41.51Z" fill="currentColor"/><path d="M31.15,38.95L26.47,48.07L28.30,59.65L29.05,59.74L35.24,48.69Z" fill="currentColor"/><path d="M26.48,36.31L16.72,39.45L12.92,46.40L13.54,47.18L22.49,46.09Z" fill="currentColor"/><path d="M25.05,31.15L15.93,26.47L4.35,28.30L4.26,29.05L15.31,35.24Z" fill="currentColor"/><path d="M27.69,26.48L24.55,16.72L17.60,12.92L16.82,13.54L17.91,22.49Z" fill="currentColor"/><path d="M32.85,25.05L37.53,15.93L35.70,4.35L34.95,4.26L28.76,15.31Z" fill="currentColor"/><path d="M37.52,27.69L47.28,24.55L51.08,17.60L50.46,16.82L41.51,17.91Z" fill="currentColor"/></svg><span>Maidan</span></a>
  <a class="signin" href="/ui/">Sign in</a>
</header>
<main id="content" tabindex="-1">
  <h1>${escapeHtml(title)}</h1>
  ${detail ? `<p class="lead">${escapeHtml(detail.charAt(0).toUpperCase() + detail.slice(1))}${/[.!?]$/.test(detail) ? "" : "."}</p>` : ""}
  <p><a href="/">Go to the front page</a>, <a href="/ui/">sign in to your workspace</a>, or <a href="/support">ask for help</a>.</p>
  ${ref}
</main>
<footer class="foot">
  <span>&copy; 2026 David Engelmann</span>
  <nav class="legal" aria-label="Policies">
    <a href="/privacy">Privacy</a>
    <a href="/terms">Terms</a>
    <a href="/support">Support</a>
  </nav>
</footer>
</body>
</html>
`;
}

// The operator's restart: `POST /__ops/restart` with
// `Authorization: Bearer <MAIDAN_OPS_RESTART_TOKEN>`. A running Container keeps
// the environment it started with across Worker deploys, so a deploy that
// changes settings without changing the image needs this to take effect.
// Answered by the Worker; the container never sees the request.
export const OPS_RESTART_PATH = "/__ops/restart";

export async function isOpsRestart(request, token) {
  if (!token || request.method !== "POST" || new URL(request.url).pathname !== OPS_RESTART_PATH) {
    return false;
  }
  const enc = new TextEncoder();
  const got = enc.encode(request.headers.get("authorization") ?? "");
  const want = enc.encode(`Bearer ${token}`);
  if (got.byteLength !== want.byteLength) return false;
  // Constant time, and portable: the Workers runtime's timingSafeEqual is not
  // in Node, where the tests run.
  let diff = 0;
  for (let i = 0; i < want.byteLength; i++) diff |= got[i] ^ want[i];
  return diff === 0;
}
