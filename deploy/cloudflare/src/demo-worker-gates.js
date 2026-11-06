// Pure gates for the try.maidan.world Worker entry (demo.js). Kept apart from
// the Containers runtime so unit tests can exercise redirects, methods and
// Host checks without wrangler.

export const DEMO_ALLOWED_METHODS = Object.freeze([
  "GET",
  "HEAD",
  "POST",
  "PUT",
  "PATCH",
  "DELETE",
  "OPTIONS",
]);

const ALLOWED = new Set(DEMO_ALLOWED_METHODS);

/** True when the method is one the demo Worker forwards or answers itself. */
export function isAllowedDemoMethod(method) {
  if (typeof method !== "string" || method.length === 0) return false;
  return ALLOWED.has(method.toUpperCase());
}

/**
 * Build the https URL for an http→https redirect. Returns null when the input
 * is not an http URL (so callers can 400 instead of bouncing a stranger onto
 * an attacker-controlled https absolute URL).
 */
export function demoHttpsRedirect(url) {
  let parsed;
  try {
    parsed = typeof url === "string" ? new URL(url) : url;
  } catch {
    return null;
  }
  if (!(parsed instanceof URL)) return null;
  if (parsed.protocol !== "http:") return null;
  if (!parsed.hostname) return null;
  const next = new URL(parsed.toString());
  next.protocol = "https:";
  // Drop userinfo so http://user:pass@host never becomes https://user:pass@host.
  next.username = "";
  next.password = "";
  return next.toString();
}

/**
 * When MAIDAN_PUBLIC_ORIGIN is set, the request Host must match that origin's
 * host (ignoring a leading www.). Missing/blank origin leaves the gate open so
 * local wrangler still works.
 */
export function demoHostAllowed(requestHost, publicOrigin) {
  if (publicOrigin == null || publicOrigin === "") return true;
  let expected;
  try {
    expected = new URL(String(publicOrigin)).hostname;
  } catch {
    return false;
  }
  if (!expected) return false;
  const host = String(requestHost ?? "")
    .split(":")[0]
    .trim()
    .toLowerCase();
  if (!host) return false;
  const want = expected.toLowerCase();
  return host === want || host === `www.${want}` || want === `www.${host}`;
}

export function demoMethodNotAllowed() {
  return new Response("method not allowed\n", {
    status: 405,
    headers: {
      allow: DEMO_ALLOWED_METHODS.join(", "),
      "content-type": "text/plain; charset=utf-8",
    },
  });
}

export function demoBadRequest(message = "bad request\n") {
  return new Response(message, {
    status: 400,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}
