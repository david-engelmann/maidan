import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  DEMO_MAX_BODY_BYTES,
  DEMO_RESET_CRON,
  DEMO_SANDBOX_MAX,
  DEMO_SERVER_ENV,
  tooLarge,
} from "../src/demo-config.js";
import { demoEnv } from "../src/demo-env.js";

const wrangler = JSON.parse(
  readFileSync(new URL("../wrangler.demo.jsonc", import.meta.url), "utf8")
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n"),
);
const demoWorker = readFileSync(new URL("../src/demo.js", import.meta.url), "utf8");
const entrypoint = readFileSync(new URL("../demo/entrypoint.sh", import.meta.url), "utf8");
const dockerfile = readFileSync(new URL("../demo/Dockerfile", import.meta.url), "utf8");
const sandboxRs = readFileSync(new URL("../../../crates/maidan-server/src/sandbox.rs", import.meta.url), "utf8");

test("the demo serves try.maidan.world from its own Worker, never a production name", () => {
  assert.equal(wrangler.name, "maidan-demo");
  assert.deepEqual(wrangler.routes, [{ pattern: "try.maidan.world", custom_domain: true }]);
  assert.equal(wrangler.main, "src/demo.js");
  assert.equal(wrangler.containers[0].max_instances, 1);
});

test("the reset cron in wrangler matches the one the config names", () => {
  assert.deepEqual(wrangler.triggers.crons, [DEMO_RESET_CRON]);
  assert.match(demoWorker, /\.destroy\(\)/);
});

test("sandbox tokens expire within a reset and a half", () => {
  const ttl = Number(sandboxRs.match(/pub const TOKEN_TTL_HOURS: i64 = (\d+);/)?.[1]);
  const hours = 24; // DEMO_RESET_CRON runs once a day; cost.test.js holds that.
  assert.ok(ttl > 0 && ttl <= hours * 1.5, `sandbox tokens live ${ttl} hours`);
});

test("the container runs the per-visitor sandbox, marked as a throwaway instance", () => {
  assert.equal(DEMO_SERVER_ENV.MAIDAN_DEMO_SANDBOX, "1");
  // Production mode refuses the mint without this marker.
  assert.equal(DEMO_SERVER_ENV.MAIDAN_ENV, "production");
  assert.equal(DEMO_SERVER_ENV.MAIDAN_DEMO_INSTANCE, "1");
  assert.equal(DEMO_SERVER_ENV.MAIDAN_DEMO_SANDBOX_MAX, String(DEMO_SANDBOX_MAX));
  assert.ok(Number(DEMO_SERVER_ENV.MAIDAN_DEMO_SANDBOX_RATE_LIMIT_MAX) > 0);
  assert.ok(Number(DEMO_SERVER_ENV.MAIDAN_DEMO_SANDBOX_RATE_LIMIT_MAX) <= 5);
});

test("each sandbox is cut off at a small stored-bytes allowance, and all of them fit the disk", () => {
  const stored = Number(DEMO_SERVER_ENV.MAIDAN_FREE_ALLOWANCE.match(/stored_bytes=(\d+)/)?.[1]);
  assert.ok(stored > 0 && stored <= 8 * 1024 * 1024, `stored_bytes=${stored}`);
  // A basic instance has 4 GB of disk; the sandboxes' content stays under a quarter.
  assert.ok(stored * DEMO_SANDBOX_MAX <= 1024 * 1024 * 1024);
  assert.ok(Number(DEMO_SERVER_ENV.MAIDAN_MAX_BODY_BYTES) <= DEMO_MAX_BODY_BYTES);
  assert.ok(Number(DEMO_SERVER_ENV.MAIDAN_ARTIFACT_MAX_BODY_BYTES) <= DEMO_MAX_BODY_BYTES);
});

test("the container has no internet and no production secret", () => {
  assert.match(demoWorker, /enableInternet = false/);
  const env = demoEnv(undefined);
  for (const name of ["S3_ACCESS_KEY_ID", "MAIDAN_SLACK_BOT_TOKEN", "MAIDAN_OIDC_CLIENT_SECRET", "MAIDAN_CONTENT_KEK"]) {
    assert.equal(env[name], undefined, name);
  }
  assert.match(env.DATABASE_URL, /^sqlite:\/\/\/tmp\//);
  assert.equal(env.MAIDAN_PUBLIC_ORIGIN, "https://try.maidan.world");
});

test("rate limits and meters are on, sign-up and bootstrap are off", () => {
  assert.ok(Number(DEMO_SERVER_ENV.MAIDAN_RATE_LIMIT_MAX) > 0);
  assert.match(DEMO_SERVER_ENV.MAIDAN_FREE_ALLOWANCE, /requests=\d+/);
  assert.match(DEMO_SERVER_ENV.MAIDAN_FREE_ALLOWANCE, /mcp_calls=\d+/);
  assert.equal(DEMO_SERVER_ENV.MAIDAN_SIGNUP, undefined);
  assert.equal(DEMO_SERVER_ENV.MAIDAN_BOOTSTRAP, undefined);
  assert.equal(DEMO_SERVER_ENV.MAIDAN_DEV_ADMIN_TOKEN, undefined);
});

test("nothing is seeded and bootstrap is never on: the image is the stock server", () => {
  const code = entrypoint.split("\n").filter((line) => !line.trim().startsWith("#")).join("\n");
  assert.doesNotMatch(code, /MAIDAN_BOOTSTRAP|demo-board|maidan init/);
  assert.match(entrypoint, /\nexec maidan-server\n/);
  const build = dockerfile.split("\n").filter((line) => !line.trim().startsWith("#")).join("\n");
  assert.doesNotMatch(build, /:\$\{?TAG\}?-demo|maidan-cli|demo-board/);
  assert.match(build, /^ARG SERVER_IMAGE$/m);
});

test("an oversized body is refused before it wakes the container", () => {
  const big = new Request("https://try.maidan.world/mcp", {
    method: "POST",
    headers: { "content-length": String(DEMO_MAX_BODY_BYTES + 1) },
  });
  const small = new Request("https://try.maidan.world/mcp", {
    method: "POST",
    headers: { "content-length": "10" },
  });
  assert.equal(tooLarge(big), true);
  assert.equal(tooLarge(small), false);
});
