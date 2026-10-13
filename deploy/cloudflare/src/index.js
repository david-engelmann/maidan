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

// Worker secrets and vars the server reads. Anything else in `env` (the
// Durable Object binding) stays in the Worker.
const SERVER_ENV = [
  "DATABASE_URL",
  "MAIDAN_CONTENT_KEK",
  "MAIDAN_SESSION_SECRET",
  "MAIDAN_PUBLIC_ORIGIN",
  "ARTIFACT_BACKEND",
  "S3_ENDPOINT",
  "S3_BUCKET",
  "S3_REGION",
  "S3_ACCESS_KEY_ID",
  "S3_SECRET_ACCESS_KEY",
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
    // A probe would wake a sleeping container only to be told 404.
    if (isScannerProbe(new URL(request.url).pathname)) {
      return new Response("not found\n", { status: 404 });
    }
    // A client-sent X-Forwarded-For would let it pick the address it is rate
    // limited as; replace it with the address Cloudflare connected from.
    const headers = new Headers(request.headers);
    headers.set("x-forwarded-for", request.headers.get("cf-connecting-ip") ?? "");
    headers.set("x-forwarded-proto", "https");
    return getContainer(env.MAIDAN).fetch(new Request(request, { headers }));
  },

  // Schedules, retries and sweeps run only while the container is awake. The
  // cron wakes it so they are at most an hour late on an idle instance.
  async scheduled(_controller, env, ctx) {
    ctx.waitUntil(getContainer(env.MAIDAN).fetch("http://maidan/health"));
  },
};
