import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  DEMO_ALLOWED_METHODS,
  demoBadRequest,
  demoHostAllowed,
  demoHttpsRedirect,
  demoMethodNotAllowed,
  isAllowedDemoMethod,
} from "../src/demo-worker-gates.js";

const demoSrc = readFileSync(new URL("../src/demo.js", import.meta.url), "utf8");

test("only the documented demo methods are allowed", () => {
  for (const m of DEMO_ALLOWED_METHODS) {
    assert.equal(isAllowedDemoMethod(m), true);
    assert.equal(isAllowedDemoMethod(m.toLowerCase()), true);
  }
  assert.equal(isAllowedDemoMethod("TRACE"), false);
  assert.equal(isAllowedDemoMethod("CONNECT"), false);
  assert.equal(isAllowedDemoMethod(""), false);
  assert.equal(isAllowedDemoMethod(null), false);
});

test("method not allowed names the Allow list", async () => {
  const res = demoMethodNotAllowed();
  assert.equal(res.status, 405);
  assert.equal(res.headers.get("allow"), DEMO_ALLOWED_METHODS.join(", "));
  assert.match(await res.text(), /method not allowed/);
});

test("http to https redirect keeps host/path/query and strips userinfo", () => {
  assert.equal(
    demoHttpsRedirect("http://try.maidan.world/try?x=1"),
    "https://try.maidan.world/try?x=1",
  );
  assert.equal(
    demoHttpsRedirect("http://user:pass@try.maidan.world/"),
    "https://try.maidan.world/",
  );
  assert.equal(demoHttpsRedirect("https://try.maidan.world/"), null);
  assert.equal(demoHttpsRedirect("ftp://try.maidan.world/"), null);
  assert.equal(demoHttpsRedirect("not a url"), null);
});

test("Host must match MAIDAN_PUBLIC_ORIGIN when set", () => {
  assert.equal(demoHostAllowed("try.maidan.world", "https://try.maidan.world"), true);
  assert.equal(demoHostAllowed("www.try.maidan.world", "https://try.maidan.world"), true);
  assert.equal(demoHostAllowed("evil.example", "https://try.maidan.world"), false);
  assert.equal(demoHostAllowed("try.maidan.world", ""), true);
  assert.equal(demoHostAllowed("try.maidan.world", null), true);
  assert.equal(demoHostAllowed("try.maidan.world", "://bad"), false);
});

test("bad request helper is 400 plain text", async () => {
  const res = demoBadRequest("host not allowed\n");
  assert.equal(res.status, 400);
  assert.equal(await res.text(), "host not allowed\n");
});

test("demo.js wires the gates before scanner and body checks", () => {
  assert.match(demoSrc, /demoHttpsRedirect/);
  assert.match(demoSrc, /isAllowedDemoMethod/);
  assert.match(demoSrc, /demoHostAllowed/);
  // Order inside fetch(), after the import block.
  const body = demoSrc.slice(demoSrc.indexOf("async fetch(request, env)"));
  const httpsIdx = body.indexOf("demoHttpsRedirect");
  const methodIdx = body.indexOf("isAllowedDemoMethod");
  const hostIdx = body.indexOf("demoHostAllowed");
  const scannerIdx = body.indexOf("isScannerProbe");
  assert.ok(httpsIdx > 0 && methodIdx > httpsIdx && hostIdx > methodIdx && scannerIdx > hostIdx);
});
