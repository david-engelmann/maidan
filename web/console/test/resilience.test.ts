// What a person sees when the server is down, refuses, or asks them to wait:
// one wording from every screen's API, and a 401 sends the page to sign-in.
import { test } from "node:test";
import assert from "node:assert/strict";
import { ApiError, OFFLINE, SIGNED_OUT, retryAfterSeconds, type Credentials } from "../src/core/api.ts";
import { BoardApi, BoardError } from "../src/core/board.ts";
import { NeedsApi } from "../src/core/needs.ts";
import { PeopleApi } from "../src/core/people.ts";
import { ThreadsApi } from "../src/core/threads.ts";
import { ToolsApi } from "../src/core/tools.ts";
import { SessionStore } from "../src/core/session.ts";

const creds = (): Credentials => ({ base: "https://m.example", pastedToken: "", tokenSession: false, sessionMemberId: "m1" });
const json = (status: number, body: unknown, h: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json", ...h } });

test("an unreachable server reads as offline, not a raw TypeError", async () => {
  const api = new BoardApi(async () => { throw new TypeError("Failed to fetch"); }, creds, () => "session");
  await assert.rejects(api.channels("w"), (e: unknown) => e instanceof BoardError && e.message === OFFLINE && e.status === 0);
});

test("a 429 names how long to wait from Retry-After", async () => {
  const api = new ThreadsApi(async () => json(429, { error: "slow down" }, { "Retry-After": "30" }), creds, () => "session");
  await assert.rejects(api.messages("t"), (e: unknown) => e instanceof ApiError && e.message === "Too many requests. Try again in 30 seconds" && e.retryAfter === 30);
});

test("Retry-After reads seconds and HTTP dates", () => {
  assert.equal(retryAfterSeconds("5"), 5);
  assert.equal(retryAfterSeconds(new Date(1_000_000 + 90_000).toUTCString(), 1_000_000), 90);
  assert.equal(retryAfterSeconds("soon"), null);
  assert.equal(retryAfterSeconds(null), null);
});

test("a 5xx never shows the server's own text", async () => {
  const api = new ToolsApi(async () => json(500, { error: "panic at db.rs:12" }), creds, () => "session");
  await assert.rejects(api.channels("w"), (e: unknown) => e instanceof ApiError && !/db\.rs/.test((e as Error).message) && (e as ApiError).status === 500);
});

test("a 401 from any screen sends a signed-in page back to sign-in with a reason", async () => {
  let authed = true;
  const fetchFn = async (url: string) => {
    if (url.endsWith("/auth/session")) return authed ? json(200, { member_id: "m1", workspace_id: "w1" }) : json(401, {});
    if (url.endsWith("/me")) return json(200, { member_id: "m1", workspace_id: "w1", is_bearer: false });
    return json(401, { error: "expired" });
  };
  const store = new SessionStore("https://m.example", fetchFn);
  const unwatch = store.watchAuth();
  await store.refresh();
  assert.equal(store.getSnapshot().status, "signed-in");
  authed = false;
  const api = new BoardApi(fetchFn, () => store.getSnapshot().credentials, () => store.getSnapshot().mode);
  await assert.rejects(api.channels("w1"));
  assert.equal(store.getSnapshot().status, "signed-out");
  assert.equal(store.getSnapshot().error, SIGNED_OUT);
  unwatch();
});

test("a 401 on a People read or a review packet also sends the page back to sign-in", async () => {
  for (const read of [
    (f: typeof fetch, s: SessionStore) => new PeopleApi(f, () => s.getSnapshot().credentials, () => s.getSnapshot().mode).people("w1").catch(() => null),
    (f: typeof fetch, s: SessionStore) => new NeedsApi(f, () => s.getSnapshot().credentials, () => s.getSnapshot().mode).reviewPacket("t1"),
  ]) {
    let authed = true;
    const fetchFn = async (url: string) => {
      if (url.endsWith("/auth/session")) return authed ? json(200, { member_id: "m1", workspace_id: "w1" }) : json(401, {});
      if (url.endsWith("/me")) return json(200, { member_id: "m1", workspace_id: "w1", is_bearer: false });
      return json(401, { error: "expired" });
    };
    const store = new SessionStore("https://m.example", fetchFn);
    const unwatch = store.watchAuth();
    await store.refresh();
    assert.equal(store.getSnapshot().status, "signed-in");
    authed = false;
    await read(fetchFn as typeof fetch, store);
    assert.equal(store.getSnapshot().status, "signed-out");
    unwatch();
  }
});

test("a 401 while signed out leaves the sign-in card alone", async () => {
  const store = new SessionStore("https://m.example", async () => json(401, {}));
  const unwatch = store.watchAuth();
  await store.refresh();
  const api = new BoardApi(async () => json(401, {}), creds, () => null);
  await assert.rejects(api.channels("w"));
  assert.equal(store.getSnapshot().error, null);
  unwatch();
});
