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
import { fetchDemo } from "./demo-edge.js";
import { ContainerEnvSession, envFingerprint } from "./server-env.js";

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
    this.envSession = new ContainerEnvSession();
  }

  // The container keeps the environment of the start() that created it. A
  // deploy can leave the new image running on the previous start's
  // environment, so a setting that arrived in the same deploy never reaches
  // the process. The session recycles that container once, and keeps a start
  // already in flight on this instance from being recycled by the next request.
  async fetch(request) {
    const fingerprint = await envFingerprint(this.envVars);
    return this.envSession.serve({
      fingerprint,
      container: this.ctx.container,
      storage: this.ctx.storage,
      blockConcurrencyWhile: (fn) => this.ctx.blockConcurrencyWhile(fn),
      destroy: async () => {
        console.log("recycling demo container to apply environment changes");
        await this.destroy();
      },
      forward: () => super.fetch(request),
    });
  }

  // The cron's reset. It destroys a running container and never starts one:
  // a sleeping demo stays asleep until a visitor wakes it. The session lock
  // is held across the destroy so a request cannot verify the container this
  // reset is removing. In-memory verified state is dropped first; a request
  // already past the lock does not record that process when it returns. The
  // next request starts with the environment this deploy is passing in.
  async reset() {
    await this.ctx.blockConcurrencyWhile(async () => {
      this.envSession.invalidate();
      if (this.ctx.container.running) await this.destroy();
    });
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
    return fetchDemo(request, (req) => getContainer(env.MAIDAN_DEMO).fetch(req));
  },

  // The reset: destroy the container and its SQLite file, and with it every
  // sandbox. The next request boots a fresh, empty one.
  async scheduled(_controller, env, ctx) {
    ctx.waitUntil(getContainer(env.MAIDAN_DEMO).reset());
  },
};
