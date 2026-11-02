// The public status page at `/status`, drawn by the Worker from the
// container's `/health`. It names each part as up or having trouble and never
// shows the server's error text, which can carry hostnames. The page runs no
// script and loads only its own stylesheet at `/status.css`. Kept apart from
// the Worker so it is testable without the Workers runtime.

export const STATUS_PATHS = new Set(["/status", "/status.css"]);

export const STATUS_CSP =
  "default-src 'none'; style-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

// How long the page waits on a waking container before it says so.
export const HEALTH_TIMEOUT_MS = 8000;

// How long one reading of `/health` answers the page. The page is public and
// every reading wakes a sleeping container, so without this a crawler hitting
// `/status` would keep the instance awake (and billed) around the clock.
export const STATUS_TTL_MS = 60_000;

// The last reading, per isolate: { at, summary }.
let lastReading = null;

// For tests: forget the last reading.
export function resetStatusCache() {
  lastReading = null;
}

const PARTS = [
  ["db", "Database"],
  ["storage", "Artifact storage"],
  ["bus", "Live updates"],
  ["indexer", "Search index"],
];

export const STATUS_CSS = `body{margin:0;font:16px/1.5 system-ui,-apple-system,sans-serif;background:#f6f3ec;color:#1f1d1a}
main{max-width:40rem;margin:0 auto;padding:3rem 1.25rem}
h1{font-size:1.6rem;letter-spacing:-.02em;margin:0 0 .25rem}
.overall{font-size:1.15rem;font-weight:600;margin:0 0 1.5rem}
.ok{color:#1e6b3a}.bad{color:#9a2a1a}
ul{list-style:none;padding:0;margin:0;border-top:1px solid #ddd6c8}
li{display:flex;justify-content:space-between;padding:.75rem 0;border-bottom:1px solid #ddd6c8}
footer{margin-top:1.5rem;color:#6b665c;font-size:.875rem}
@media (prefers-color-scheme:dark){body{background:#17160f;color:#ece7dc}ul,li{border-color:#3a372f}footer{color:#a39d90}.ok{color:#6fcf8f}.bad{color:#f08a76}}`;

// Whether a request is for the status page or its stylesheet.
export function isStatusRequest(request) {
  if (request.method !== "GET" && request.method !== "HEAD") {
    return false;
  }
  return STATUS_PATHS.has(new URL(request.url).pathname);
}

const escape = (s) =>
  String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

// What `/health` said, reduced to what the public may see: overall up or not,
// each part up or not, and the version. `health` is null when the container
// did not answer.
export function summarize(health) {
  if (!health || typeof health !== "object") {
    return { up: false, reachable: false, parts: [], version: null };
  }
  const parts = PARTS.filter(([key]) => key in health).map(([key, label]) => ({ label, ok: health[key] === "ok" }));
  return {
    up: health.status === "ok",
    reachable: true,
    parts,
    version: typeof health.version === "string" ? health.version : null,
  };
}

export function renderStatus(summary, now = new Date()) {
  const overall = !summary.reachable
    ? "Maidan is not answering right now."
    : summary.up
      ? "All systems working."
      : "Some parts are having trouble.";
  const rows = summary.parts
    .map((p) => `<li><span>${escape(p.label)}</span><span class="${p.ok ? "ok" : "bad"}">${p.ok ? "Working" : "Trouble"}</span></li>`)
    .join("");
  const version = summary.version ? ` · version ${escape(summary.version)}` : "";
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Maidan status</title><link rel="stylesheet" href="/status.css"></head>
<body><main><h1>Maidan status</h1>
<p class="overall ${summary.up ? "ok" : "bad"}">${overall}</p>
${rows ? `<ul>${rows}</ul>` : ""}
<footer>Checked ${escape(now.toISOString().replace(/\.\d+Z$/, "Z"))}${version}</footer>
</main></body></html>
`;
}

// The answer for `/status` or `/status.css`. `fetchHealth` returns the
// container's `/health` Response (or throws).
export async function statusResponse(
  request,
  fetchHealth,
  { timeoutMs = HEALTH_TIMEOUT_MS, ttlMs = STATUS_TTL_MS, now } = {},
) {
  const path = new URL(request.url).pathname;
  if (path === "/status.css") {
    return new Response(STATUS_CSS, {
      headers: { "content-type": "text/css; charset=utf-8", "cache-control": "public, max-age=3600" },
    });
  }
  const at = (now ?? new Date()).getTime();
  if (lastReading && at - lastReading.at < ttlMs) {
    return page(request, lastReading.summary, new Date(lastReading.at));
  }
  let health = null;
  let timer;
  try {
    const res = await Promise.race([
      fetchHealth(),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("timeout")), timeoutMs);
      }),
    ]);
    // `/health` answers 503 with the same body when a part is down.
    health = await res.json();
  } catch {
    health = null;
  } finally {
    clearTimeout(timer);
  }
  const summary = summarize(health);
  lastReading = { at, summary };
  return page(request, summary, new Date(at));
}

function page(request, summary, checkedAt) {
  return new Response(request.method === "HEAD" ? null : renderStatus(summary, checkedAt), {
    status: 200,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "public, max-age=60",
      "content-security-policy": STATUS_CSP,
    },
  });
}
