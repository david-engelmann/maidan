// What try.maidan.world answers when the demo container cannot. Kept apart
// from demo.js so the tests run it without the Containers runtime.
//
// A cold start, a deploy's replacement and the daily reset all surface as a
// thrown fetch or the runtime's plain-text "Error proxying request to
// container". Visitors should get the same overloaded problem the hosted
// Worker sends, not that text. The try page's assets are `./try` (try.css),
// not the hosted site's stylesheet.

import {
  isContainerUnavailable,
  isPageNavigation,
  UNAVAILABLE_RETRY_SECS,
  unavailableProblem,
  withSecurityHeaders,
} from "./edge.js";

// No script: the page only loads the try stylesheet, which pulls its font
// from the same origin. Nothing inline, which the try page's CSP would block.
export const DEMO_UNAVAILABLE_CSP =
  "default-src 'none'; style-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

export function demoUnavailablePage() {
  const seconds = UNAVAILABLE_RETRY_SECS;
  const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="color-scheme" content="light">
<meta name="theme-color" content="#f2eee6">
<meta name="referrer" content="no-referrer">
<title>Back in a moment · Maidan</title>
<meta http-equiv="refresh" content="${seconds}">
<link rel="stylesheet" href="/try.css">
</head>
<body>
<a class="skip" href="#main">Skip to content</a>
<header class="top">
  <a class="brand" href="/" aria-label="Maidan home">Maidan</a>
  <nav class="top-nav" aria-label="Demo">
    <a class="top-link" href="https://docs.maidan.world/">Docs</a>
  </nav>
</header>
<main id="main">
  <section>
    <p class="eyebrow">Live demo</p>
    <h1>Back in a moment.</h1>
    <p class="lede">The demo is starting or being updated. This usually takes under a minute, and this page tries again every ${seconds} seconds.</p>
    <p class="notice"><a href="/">Back to the try page</a></p>
  </section>
</main>
</body>
</html>
`;
  return new Response(html, {
    status: 503,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "retry-after": String(seconds),
    },
  });
}

function unavailableDemo(request) {
  if (isPageNavigation(request)) {
    return withSecurityHeaders(demoUnavailablePage(), {
      "content-security-policy": DEMO_UNAVAILABLE_CSP,
    });
  }
  return withSecurityHeaders(unavailableProblem());
}

// Proxy one request to the demo container. `fetchContainer` is
// `getContainer(env.MAIDAN_DEMO).fetch`. A WebSocket upgrade is returned as
// the runtime handed it over: rebuilding it would drop the socket.
export async function fetchDemo(request, fetchContainer) {
  const headers = new Headers(request.headers);
  headers.set("x-forwarded-for", request.headers.get("cf-connecting-ip") ?? "");
  headers.set("x-forwarded-proto", "https");
  let response;
  try {
    response = await fetchContainer(new Request(request, { headers }));
  } catch (err) {
    console.error("demo container fetch failed", err);
    response = null;
  }
  if (response !== null && (response.status === 101 || response.webSocket)) {
    return response;
  }
  if (response === null || (await isContainerUnavailable(response))) {
    return unavailableDemo(request);
  }
  return withSecurityHeaders(response);
}
