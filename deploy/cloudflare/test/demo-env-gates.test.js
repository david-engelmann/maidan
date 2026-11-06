import { test } from "node:test";
import assert from "node:assert/strict";
import {
  DEMO_DEFAULT_PUBLIC_ORIGIN,
  demoEnv,
  normalizePublicOrigin,
} from "../src/demo-env.js";

test("https origins are reduced to scheme+host(+port) with no credentials", () => {
  assert.equal(normalizePublicOrigin("https://try.maidan.world/path?x=1#y"), "https://try.maidan.world");
  assert.equal(normalizePublicOrigin(" https://Try.Maidan.World "), "https://try.maidan.world");
  assert.equal(normalizePublicOrigin("https://try.maidan.world:8443/app"), "https://try.maidan.world:8443");
});

test("http, credential-bearing, and malformed origins fall back to the try host", () => {
  for (const bad of [
    "http://try.maidan.world",
    "https://user:pass@try.maidan.world",
    "javascript:alert(1)",
    "not a url",
    "",
    "   ",
    null,
    undefined,
  ]) {
    assert.equal(normalizePublicOrigin(bad), DEMO_DEFAULT_PUBLIC_ORIGIN, String(bad));
  }
});

test("demoEnv always publishes a safe https origin", () => {
  assert.equal(demoEnv("http://evil.example").MAIDAN_PUBLIC_ORIGIN, DEMO_DEFAULT_PUBLIC_ORIGIN);
  assert.equal(demoEnv("https://sandbox.example.com/x").MAIDAN_PUBLIC_ORIGIN, "https://sandbox.example.com");
  assert.equal(demoEnv(undefined).MAIDAN_PUBLIC_ORIGIN, DEMO_DEFAULT_PUBLIC_ORIGIN);
});
