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

// One script, the console's pre-paint theme.js (served by the Worker beside
// the try page), so the page keeps the visitor's stored theme; then the try
// stylesheet, which pulls its font from the same origin. Nothing inline,
// which the try page's CSP would block.
export const DEMO_UNAVAILABLE_CSP =
  "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

export function demoUnavailablePage() {
  const seconds = UNAVAILABLE_RETRY_SECS;
  const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="referrer" content="no-referrer">
<meta name="color-scheme" content="dark light">
<meta name="theme-color" content="#0c0d0f">
<script src="/theme.js"></script>
<title>Back in a moment · Maidan</title>
<meta http-equiv="refresh" content="${seconds}">
<link rel="stylesheet" href="/try.css">
</head>
<body>
<a class="skip" href="#main">Skip to content</a>
<header class="top">
  <a class="brand" href="/"><svg class="mark" viewBox="0 0 64 64" aria-hidden="true"><path d="M38.95,32.85L48.07,37.53L59.65,35.70L59.74,34.95L48.69,28.76Z" fill="currentColor"/><path d="M36.31,37.52L39.45,47.28L46.40,51.08L47.18,50.46L46.09,41.51Z" fill="currentColor"/><path d="M31.15,38.95L26.47,48.07L28.30,59.65L29.05,59.74L35.24,48.69Z" fill="currentColor"/><path d="M26.48,36.31L16.72,39.45L12.92,46.40L13.54,47.18L22.49,46.09Z" fill="currentColor"/><path d="M25.05,31.15L15.93,26.47L4.35,28.30L4.26,29.05L15.31,35.24Z" fill="currentColor"/><path d="M27.69,26.48L24.55,16.72L17.60,12.92L16.82,13.54L17.91,22.49Z" fill="currentColor"/><path d="M32.85,25.05L37.53,15.93L35.70,4.35L34.95,4.26L28.76,15.31Z" fill="currentColor"/><path d="M37.52,27.69L47.28,24.55L51.08,17.60L50.46,16.82L41.51,17.91Z" fill="currentColor"/></svg><span>Maidan</span></a>
  <a class="top-link" href="https://docs.maidan.world/">Docs</a>
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
