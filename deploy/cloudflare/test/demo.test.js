import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  DEMO_MAX_BODY_BYTES,
  DEMO_RESET_CRON,
  DEMO_SERVER_ENV,
  DEMO_TOKEN_TTL_SECS,
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

test("seeded tokens expire within a reset and a half", () => {
  assert.equal(DEMO_SERVER_ENV.DEMO_TOKEN_TTL_SECS, String(DEMO_TOKEN_TTL_SECS));
  const hours = Number(DEMO_RESET_CRON.split(" ")[1].replace("*/", ""));
  assert.ok(DEMO_TOKEN_TTL_SECS > hours * 3600);
  assert.ok(DEMO_TOKEN_TTL_SECS <= hours * 3600 * 1.5);
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
});

test("bootstrap is on only for the loopback seed listener", () => {
  const lines = entrypoint.split("\n").filter((line) => !line.trim().startsWith("#") && line.includes("MAIDAN_BOOTSTRAP=1"));
  assert.equal(lines.length, 1);
  assert.match(lines[0], /MAIDAN_BIND="127\.0\.0\.1:/);
  assert.match(entrypoint, /\nexec maidan-server\n/);
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
