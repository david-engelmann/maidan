// What the Worker decides before and after the container: plain HTTP goes to
// HTTPS, a browser opening the bare origin of an instance without a site
// lands in the console, and every answer carries the headers that keep a browser on HTTPS and off content
// sniffing. Kept apart from the Worker so it is testable without the Workers
// runtime.

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
