// Ported from the board's helpers.test.mjs: the path split, the refusals,
// credential modes and error wording, now against the console's typed core.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  apiPath,
  bearerRefusal,
  credentialMode,
  humanError,
  signOutPostsLogout,
  writeInit,
  writeRefusal,
  wsUrl,
  type Credentials,
} from "../src/core/api.ts";

const BASE = "https://maidan.example";
const none: Credentials = { base: `${BASE}/`, pastedToken: "", tokenSession: false, sessionMemberId: null };

test("a session read goes through /ui/api and a pasted token goes to the bearer API", () => {
  assert.equal(apiPath(none, "/threads/t1"), `${BASE}/ui/api/threads/t1`);
  assert.equal(apiPath({ ...none, pastedToken: "  " }, "/me"), `${BASE}/ui/api/me`);
  assert.equal(apiPath({ ...none, pastedToken: "sekrit" }, "/threads/t1"), `${BASE}/threads/t1`);
});

test("a session made from a token uses the bearer path with the field empty", () => {
  const c = { ...none, tokenSession: true, sessionMemberId: "mem_1" };
  assert.equal(apiPath(c, "/workspaces/ws_1"), `${BASE}/workspaces/ws_1`);
});

test("bearerRefusal allows a token and tells a session it cannot call that route", () => {
  assert.equal(bearerRefusal(none), "Set a bearer token to do this.");
  assert.equal(bearerRefusal({ ...none, pastedToken: "sekrit" }), null);
  assert.equal(
    bearerRefusal({ ...none, sessionMemberId: "mem_1" }),
    "This needs a bearer token. A signed-in session cannot call it.",
  );
});

test("a write with no credential points at Connect this browser", () => {
  assert.equal(writeRefusal(none, null), "Sign in first: paste a token under Connect this browser.");
  assert.equal(
    writeRefusal(none, "/auth/oidc/login"),
    "Sign in first: paste a token under Connect this browser, or sign in with your identity provider.",
  );
  assert.equal(writeRefusal({ ...none, pastedToken: "sekrit" }, null), null);
  assert.equal(writeRefusal({ ...none, sessionMemberId: "mem_1" }, null), null);
});

test("sign out posts logout when the server offers sessions, even with no cached member", () => {
  assert.equal(signOutPostsLogout(true, null), true);
  assert.equal(signOutPostsLogout(false, null), false);
  assert.equal(signOutPostsLogout(false, "mem_1"), true);
  assert.equal(signOutPostsLogout(null, null), true);
  assert.equal(signOutPostsLogout(undefined, null), true);
});

test("credentialMode reads session, bearer, or delegated from /me", () => {
  assert.equal(credentialMode(null), null);
  assert.equal(credentialMode({ is_bearer: false, delegation_grant_id: null }), "session");
  assert.equal(credentialMode({ is_bearer: true, delegation_grant_id: null }), "bearer");
  assert.equal(credentialMode({ is_bearer: true, delegation_grant_id: "grant_1" }), "delegated");
});

test("humanError names the capability and never the status or the raw body", () => {
  assert.equal(humanError(401, "unauthorized", null), "Your token or session was not accepted. Use Change to set a working one");
  assert.match(humanError(403, "needs approval:grant", "bearer"), /it needs approval:grant/);
  assert.match(humanError(401, "", "session"), /^Your session was not accepted/);
  assert.doesNotMatch(humanError(403, "", "session"), /token is not/);
  for (const status of [400, 401, 403, 404, 409, 413, 429, 500, 503]) {
    const said = humanError(status, "raw server body", null);
    assert.equal(said.includes("raw server body"), false);
    assert.equal(said.includes(String(status)), false);
  }
});

test("each write carries its own idempotency key and keeps its headers", () => {
  const a = writeInit({ method: "POST", headers: { Accept: "application/json" } });
  const b = writeInit({ method: "POST" });
  const ha = a.headers as Record<string, string>;
  const hb = b.headers as Record<string, string>;
  assert.equal(ha.Accept, "application/json");
  assert.ok(ha["Idempotency-Key"]);
  assert.notEqual(ha["Idempotency-Key"], hb["Idempotency-Key"]);
});

test("the socket URL follows the page's scheme", () => {
  assert.equal(wsUrl("https://maidan.example/"), "wss://maidan.example/ws/subscribe");
  assert.equal(wsUrl("http://127.0.0.1:8080"), "ws://127.0.0.1:8080/ws/subscribe");
});
