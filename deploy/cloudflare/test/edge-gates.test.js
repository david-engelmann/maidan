import { test } from "node:test";
import assert from "node:assert/strict";
import {
  tryPathname,
  sanitizeTurnstileToken,
  turnstileAccepted,
  handleTry,
  TURNSTILE_ACTION,
  TURNSTILE_SITE_KEY_VAR,
  TURNSTILE_SECRET_VAR,
} from "../src/try-page.js";
import { isScannerProbe, normalizeScannerPath } from "../src/scanner.js";
import { tooLarge, DEMO_MAX_BODY_BYTES } from "../src/demo-config.js";

test("tryPathname strips a trailing slash except on the root", () => {
  assert.equal(tryPathname("/"), "/");
  assert.equal(tryPathname("/try.css/"), "/try.css");
  assert.equal(tryPathname("/try-config.json/"), "/try-config.json");
  assert.equal(tryPathname("/sandbox/"), "/sandbox");
  assert.equal(tryPathname(""), "/");
});

test("sanitizeTurnstileToken drops empty, oversized, and control-bearing tokens", () => {
  assert.equal(sanitizeTurnstileToken("  tok  "), "tok");
  assert.equal(sanitizeTurnstileToken(""), "");
  assert.equal(sanitizeTurnstileToken("a".repeat(2049)), "");
  assert.equal(sanitizeTurnstileToken("to\nk"), "");
  assert.equal(sanitizeTurnstileToken(null), "");
  assert.equal(sanitizeTurnstileToken(12), "");
});

test("turnstileAccepted requires success, matching host (case-insensitive), and action", () => {
  assert.equal(
    turnstileAccepted({ success: true, hostname: "Try.Maidan.World", action: TURNSTILE_ACTION }, "try.maidan.world"),
    true,
  );
  assert.equal(
    turnstileAccepted({ success: true, hostname: "try.maidan.world", action: "waitlist" }, "try.maidan.world"),
    false,
  );
  assert.equal(
    turnstileAccepted({ success: true, hostname: "evil.example", action: TURNSTILE_ACTION }, "try.maidan.world"),
    false,
  );
  assert.equal(turnstileAccepted({ success: false, hostname: "try.maidan.world", action: TURNSTILE_ACTION }, "try.maidan.world"), false);
});

test("handleTry serves try assets when the path has a trailing slash", async () => {
  const seen = [];
  const env = {
    ASSETS: {
      fetch: async (req) => {
        seen.push(new URL(req.url).pathname);
        return new Response("ok", { headers: { "content-type": "text/css" } });
      },
    },
  };
  const res = await handleTry(new Request("https://try.maidan.world/try.css/"), env);
  assert.ok(res);
  assert.equal(await res.text(), "ok");
  assert.deepEqual(seen, ["/try.css"]);
});

test("handleTry treats POST /sandbox/ as the mint path", async () => {
  const env = {
    [TURNSTILE_SITE_KEY_VAR]: "site",
    [TURNSTILE_SECRET_VAR]: "secret",
  };
  const res = await handleTry(
    new Request("https://try.maidan.world/sandbox/", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ turnstile_token: "tok" }),
    }),
    env,
    async () => new Response(JSON.stringify({ success: true, hostname: "try.maidan.world", action: TURNSTILE_ACTION }), {
      headers: { "content-type": "application/json" },
    }),
  );
  assert.equal(res, null, "accepted token returns null so the container mint runs");
});

test("a control-bearing Turnstile token is refused before siteverify", async () => {
  let called = 0;
  const env = {
    [TURNSTILE_SITE_KEY_VAR]: "site",
    [TURNSTILE_SECRET_VAR]: "secret",
  };
  const res = await handleTry(
    new Request("https://try.maidan.world/sandbox", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ turnstile_token: "bad\ntoken" }),
    }),
    env,
    async () => {
      called += 1;
      return new Response("{}");
    },
  );
  assert.equal(res.status, 403);
  assert.equal(called, 0);
});

test("scanner + tooLarge gates stay wired together for the demo edge", () => {
  assert.ok(isScannerProbe(normalizeScannerPath("/./.env")));
  assert.equal(
    tooLarge(new Request("https://try.maidan.world/mcp", {
      method: "POST",
      headers: { "content-length": String(DEMO_MAX_BODY_BYTES) },
    })),
    false,
  );
  assert.equal(
    tooLarge(new Request("https://try.maidan.world/mcp", {
      method: "POST",
      headers: { "content-length": String(DEMO_MAX_BODY_BYTES + 1) },
    })),
    true,
  );
});
