// The Worker in front of a hosted Maidan on Cloudflare Containers.
//
// One container serves every request, which keeps an idle instance at the cost
// of one; the bus runs through Postgres, so more can run when traffic needs
// them (`max_instances` and `getRandom`). The container sleeps after
// `sleepAfter` without a request and the next request wakes it. Postgres (Neon)
// and artifacts (R2) live outside it, so a sleep or a restart loses nothing but
// in-memory presence and live connections, which clients already reconnect.

import { Container, getContainer } from "@cloudflare/containers";
import { isScannerProbe } from "./scanner.js";
import {
  edgeRedirect,
  isPageNavigation,
  isSitePage,
  HOSTED_FOOTER_LINKS,
  isContainerUnavailable,
  isProblemForAPerson,
  problemPage,
  NOT_FOUND_PAGE,
  SITE_CSP,
  UNAVAILABLE_PAGE,
  UNAVAILABLE_RETRY_SECS,
  unavailableProblem,
  withSecurityHeaders,
} from "./edge.js";
import { isStatusRequest, statusResponse } from "./status.js";

// Worker secrets and vars the server reads. Anything else in `env` (the
// Durable Object binding) stays in the Worker.
const SERVER_ENV = [
  "DATABASE_URL",
  "MAIDAN_CONTENT_KEK",
  "MAIDAN_SESSION_SECRET",
  "MAIDAN_PUBLIC_ORIGIN",
  "MAIDAN_OAUTH_CLIENT_METADATA_HOSTS",
  "ARTIFACT_BACKEND",
  "S3_ENDPOINT",
  "S3_BUCKET",
  "S3_REGION",
  "S3_ACCESS_KEY_ID",
  "S3_SECRET_ACCESS_KEY",
  "MAIDAN_OIDC_ENABLED",
  "MAIDAN_OIDC_ISSUER",
  "MAIDAN_OIDC_CLIENT_ID",
  "MAIDAN_OIDC_CLIENT_SECRET",
  "MAIDAN_OIDC_REDIRECT_URI",
  "MAIDAN_OIDC_LINK_EMAIL",
  "MAIDAN_OIDC_EMAILS_VERIFIED",
  "MAIDAN_SIGNUP",
  "MAIDAN_SIGNUP_EMAIL_DOMAINS",
  "MAIDAN_SIGNUP_OPEN",
  "MAIDAN_SIGNUP_PRIVACY_URL",
  "MAIDAN_SIGNUP_TERMS_URL",
  "MAIDAN_FREE_ALLOWANCE",
  "MAIDAN_EXPORT_SIGNING_KEY",
  "MAIDAN_TURNSTILE_SECRET",
  "MAIDAN_TURNSTILE_SITE_KEY",
  "MAIDAN_GITHUB_TOKEN",
  "MAIDAN_GITHUB_WRITE_REPOS",
  "MAIDAN_OPS_ALERT_CHANNEL",
  "MAIDAN_SLACK_BOT_TOKEN",
  "MAIDAN_VAPID_PRIVATE_KEY",
  "MAIDAN_VAPID_PUBLIC_KEY",
  "MAIDAN_VAPID_SUBJECT",
];

export class MaidanServer extends Container {
  defaultPort = 8080;
  sleepAfter = "15m";

  constructor(ctx, env) {
    super(ctx, env);
    const passed = Object.fromEntries(
      SERVER_ENV.filter((name) => env[name] !== undefined).map((name) => [name, env[name]]),
    );
    this.envVars = {
      ...passed,
      MAIDAN_BIND: "0.0.0.0:8080",
      MAIDAN_ENV: "production",
      MAIDAN_LOG_FORMAT: "json",
      // The Worker rewrites X-Forwarded-For to the one address Cloudflare saw,
      // so the server trusts exactly that hop for rate limits.
      MAIDAN_TRUSTED_PROXY_HOPS: "1",
    };
  }
}

export default {
  async fetch(request, env) {
    const hasSite = env.ASSETS !== undefined;
    const redirect = edgeRedirect(request, { hasSite });
    if (redirect) {
      return redirect;
    }
    if (hasSite && isSitePage(request)) {
      return withSecurityHeaders(await env.ASSETS.fetch(request), {
        "content-security-policy": SITE_CSP,
      });
    }
    // The public status page, drawn here from the container's `/health` so it
    // can say plainly when the container does not answer.
    if (isStatusRequest(request)) {
      return withSecurityHeaders(
        await statusResponse(request, () => getContainer(env.MAIDAN).fetch("http://maidan/health")),
      );
    }
    // A probe would wake a sleeping container only to be told 404.
    if (isScannerProbe(new URL(request.url).pathname)) {
      return withSecurityHeaders(new Response("not found\n", { status: 404 }));
    }
    // A client-sent X-Forwarded-For would let it pick the address it is rate
    // limited as; replace it with the address Cloudflare connected from.
    const headers = new Headers(request.headers);
    headers.set("x-forwarded-for", request.headers.get("cf-connecting-ip") ?? "");
    headers.set("x-forwarded-proto", "https");
    let response;
    try {
      response = await getContainer(env.MAIDAN).fetch(new Request(request, { headers }));
    } catch (err) {
      console.error("container fetch failed", err);
      response = null;
    }
    // While a deploy replaces the container, or one fails to start, a person
    // sees a page that says so and retries itself, and a client gets a
    // problem it knows to retry, not the runtime's own error text.
    if (response === null || (await isContainerUnavailable(response))) {
      if (hasSite && isPageNavigation(request)) {
        const page = await env.ASSETS.fetch(new Request(new URL(UNAVAILABLE_PAGE, request.url)));
        if (page.ok) {
          return withSecurityHeaders(new Response(page.body, { status: 503, headers: page.headers }), {
            "content-security-policy": SITE_CSP,
            "retry-after": String(UNAVAILABLE_RETRY_SECS),
            "cache-control": "no-store",
          });
        }
      }
      return withSecurityHeaders(unavailableProblem());
    }
    // The waitlist page links the hosted site's support and status pages,
    // which a self-hosted instance does not have.
    const url = new URL(request.url);
    if (hasSite && url.pathname === "/" && response.ok
        && (response.headers.get("content-type") ?? "").startsWith("text/html")) {
      const links = HOSTED_FOOTER_LINKS.map(([href, label]) => `<a href="${href}">${label}</a>\n        `).join("");
      response = new HTMLRewriter()
        .on('nav.legal a[href="/ui"]', { element: (el) => el.before(links, { html: true }) })
        .transform(response);
    }
    // Any other problem body a browser was sent as a page (an expired invite,
    // a sign-in for the wrong identity) becomes a page that says it plainly.
    if (hasSite && response.status !== 404 && isProblemForAPerson(request, response)) {
      let problem = null;
      try {
        problem = await response.clone().json();
      } catch {
        problem = null;
      }
      const page = problemPage(problem, response.headers.get("x-request-id"));
      return withSecurityHeaders(
        new Response(page, {
          status: response.status,
          headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" },
        }),
        { "content-security-policy": SITE_CSP },
      );
    }
    // The server's 404 is a problem+json body, which a person who followed a
    // bad link should not be shown.
    if (hasSite && response.status === 404 && isPageNavigation(request)) {
      const page = await env.ASSETS.fetch(new Request(new URL(NOT_FOUND_PAGE, request.url)));
      if (page.ok) {
        return withSecurityHeaders(new Response(page.body, { status: 404, headers: page.headers }), {
          "content-security-policy": SITE_CSP,
        });
      }
    }
    return withSecurityHeaders(response);
  },

  // Schedules, retries and sweeps run only while the container is awake. The
  // daily cron wakes it so they are at most a day late on an idle instance;
  // an hourly one kept an idle instance awake a quarter of the day.
  async scheduled(_controller, env, ctx) {
    ctx.waitUntil(getContainer(env.MAIDAN).fetch("http://maidan/health"));
  },
};
