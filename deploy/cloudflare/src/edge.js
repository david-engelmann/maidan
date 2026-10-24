// What the Worker decides before and after the container: plain HTTP goes to
// HTTPS, a browser opening the bare origin lands in the console, and every
// answer carries the headers that keep a browser on HTTPS and off content
// sniffing. Kept apart from the Worker so it is testable without the Workers
// runtime.

// A year of HSTS for the zone's hosts, all of which serve HTTPS only.
export const SECURITY_HEADERS = {
  "strict-transport-security": "max-age=31536000; includeSubDomains",
  "x-content-type-options": "nosniff",
  "referrer-policy": "strict-origin-when-cross-origin",
};

// The redirect a request gets before it reaches the container, or null.
export function edgeRedirect(request) {
  const url = new URL(request.url);
  if (url.protocol === "http:") {
    url.protocol = "https:";
    return Response.redirect(url.toString(), 301);
  }
  // The server answers `/` for API clients; only a browser asking for a page
  // is sent to the console.
  const accept = request.headers.get("accept") ?? "";
  if (url.pathname === "/" && request.method === "GET" && accept.includes("text/html")) {
    return Response.redirect(new URL("/ui/", url).toString(), 302);
  }
  return null;
}

// The container's answer with the security headers added where it set none.
// A WebSocket upgrade is passed through as it is: rebuilding it would drop
// the socket.
export function withSecurityHeaders(response) {
  if (response.status === 101 || response.webSocket) {
    return response;
  }
  const out = new Response(response.body, response);
  for (const [name, value] of Object.entries(SECURITY_HEADERS)) {
    if (!out.headers.has(name)) {
      out.headers.set(name, value);
    }
  }
  return out;
}
