// The Worker for try.maidan.world, the public demo: a stranger starts a
// sandbox of their own on the "try" page and connects an agent to it. The
// runbook is docs/Deploy.md "try.maidan.world"; the deploy-demo workflow
// deploys it.

import { Container, getContainer } from "@cloudflare/containers";
import { isScannerProbe } from "./scanner.js";
import { withSecurityHeaders } from "./edge.js";
import { DEMO_PUBLIC_ORIGIN_VAR, demoEnv } from "./demo-env.js";
import { tooLarge } from "./demo-config.js";
import { handleTry } from "./try-page.js";

export class MaidanDemo extends Container {
  defaultPort = 8080;
  // Cost: an idle demo sleeps after 15 minutes.
  sleepAfter = "15m";
  // No outbound egress: webhooks, delivery to GitHub, Slack or Linear, and
  // OAuth metadata fetches all fail closed inside the demo.
  enableInternet = false;

  constructor(ctx, env) {
    super(ctx, env);
    this.envVars = demoEnv(env[DEMO_PUBLIC_ORIGIN_VAR]);
  }

  // The cron's reset. It destroys a running container and never starts one:
  // a sleeping demo stays asleep until a visitor wakes it.
  async reset() {
    if (this.ctx.container.running) await this.destroy();
  }
}

export default {
  async fetch(request, env) {
    if (new URL(request.url).protocol === "http:") {
      const url = new URL(request.url);
      url.protocol = "https:";
      return Response.redirect(url.toString(), 301);
    }
    if (isScannerProbe(new URL(request.url).pathname)) {
      return withSecurityHeaders(new Response("not found\n", { status: 404 }));
    }
    if (tooLarge(request)) {
      return withSecurityHeaders(new Response("request too large for the demo\n", { status: 413 }));
    }
    // The try page and its files never wake the container; a sandbox mint
    // whose human check fails never reaches it.
    const own = await handleTry(request, env);
    if (own) return withSecurityHeaders(own);
    const headers = new Headers(request.headers);
    headers.set("x-forwarded-for", request.headers.get("cf-connecting-ip") ?? "");
    headers.set("x-forwarded-proto", "https");
    return withSecurityHeaders(
      await getContainer(env.MAIDAN_DEMO).fetch(new Request(request, { headers })),
    );
  },

  // The reset: destroy the container and its SQLite file, and with it every
  // sandbox. The next request boots a fresh, empty one.
  async scheduled(_controller, env, ctx) {
    ctx.waitUntil(getContainer(env.MAIDAN_DEMO).reset());
  },
};
