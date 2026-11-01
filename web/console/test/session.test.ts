// The session store: discovery, the cookie session, and the token exchange.
import { test } from "node:test";
import assert from "node:assert/strict";
import { SessionStore, grantCard, readDiscovery, readFrontDoorHint, type Fetch } from "../src/core/session.ts";

const BASE = "https://maidan.example";
const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

test("discovery: a 404 is an answer, a 503 is not", async () => {
  assert.equal((await readDiscovery(async () => new Response("", { status: 404 }), BASE)).answered, true);
  assert.equal((await readDiscovery(async () => new Response("", { status: 503 }), BASE)).answered, false);
  const d = await readDiscovery(async () => json(200, { auth: { oidc: true, oidc_login: "/auth/oidc/login", sessions: true } }), BASE);
  assert.deepEqual([d.oidcLoginPath, d.sessions], ["/auth/oidc/login", true]);
});

test("a cookie session signs in and /me names the mode", async () => {
  const fetchFn: Fetch = async (url) => {
    if (url.endsWith("/auth/session")) return json(200, { member_id: "mem_1", token_id: null, display_name: " Ada ", workspace_id: "ws_1" });
    // The bearer /me refuses a cookie; a person's sign-in reads the /ui/api one.
    if (url === `${BASE}/me`) return json(403, { detail: "This needs a bearer token" });
    if (url === `${BASE}/ui/api/me`) return json(200, { member_id: "mem_1", workspace_id: "ws_1", is_bearer: false });
    return new Response("", { status: 404 });
  };
  const s = new SessionStore(BASE, fetchFn);
  const st = await s.refresh();
  assert.equal(st.status, "signed-in");
  assert.equal(st.displayName, "Ada");
  assert.equal(st.mode, "session");
  assert.equal(st.workspaceId, "ws_1");
});

test("a token is traded for a session and never kept in the page", async () => {
  let session = false;
  const fetchFn: Fetch = async (url, init) => {
    if (url.endsWith("/auth/session/from-token")) {
      assert.ok((init?.headers as Record<string, string>)["Idempotency-Key"]);
      session = true;
      return json(200, {});
    }
    if (url.endsWith("/auth/session"))
      return session ? json(200, { member_id: "mem_1", token_id: "tok_1", workspace_id: "ws_1" }) : json(401, {});
    if (url.endsWith("/me")) return json(200, { member_id: "mem_1", workspace_id: "ws_1", is_bearer: true });
    return json(404, {});
  };
  const s = new SessionStore(BASE, fetchFn);
  assert.deepEqual(await s.signInWithToken("sekrit"), { ok: true });
  const st = s.getSnapshot();
  assert.equal(st.credentials.pastedToken, "");
  assert.equal(st.credentials.tokenSession, true);
  assert.equal(st.mode, "bearer");
});

test("a server with no session key keeps the token in this tab only", async () => {
  const fetchFn: Fetch = async (url) => {
    if (url.endsWith("/auth/session/from-token")) return json(404, {});
    if (url.endsWith("/auth/session")) return json(404, {});
    if (url.endsWith("/me")) return json(200, { member_id: "mem_2", workspace_id: "ws_2", is_bearer: true });
    return json(404, {});
  };
  const s = new SessionStore(BASE, fetchFn);
  await s.signInWithToken("sekrit");
  const st = s.getSnapshot();
  assert.equal(st.status, "signed-in");
  assert.equal(st.credentials.pastedToken, "sekrit");
  assert.equal(st.memberId, "mem_2");
});

test("a rejected token says so without echoing the secret", async () => {
  const s = new SessionStore(BASE, async () => json(401, {}));
  const r = await s.signInWithToken("sekrit");
  assert.equal(r.ok, false);
  assert.match(r.error ?? "", /not accepted/);
  assert.equal((r.error ?? "").includes("sekrit"), false);
});

test("an overtaken read does not land", async () => {
  let release: () => void = () => {};
  const slow = new Promise<void>((r) => (release = r));
  let calls = 0;
  const fetchFn: Fetch = async (url) => {
    if (url.endsWith("/auth/session")) {
      calls++;
      if (calls === 1) {
        await slow;
        return json(200, { member_id: "old", workspace_id: "ws_old" });
      }
      return json(401, {});
    }
    return json(404, {});
  };
  const s = new SessionStore(BASE, fetchFn);
  const first = s.refresh();
  await s.refresh();
  release();
  await first;
  assert.equal(s.getSnapshot().status, "signed-out");
  assert.equal(s.getSnapshot().memberId, null);
});

test("the capability card splits the real grant from the vocabulary", () => {
  const card = grantCard({ capabilities: ["workspace:read", "message:post"], known: ["message:post", "token:admin", "workspace:read", "channel:admin"] });
  assert.deepEqual(card.can, ["message:post", "workspace:read"]);
  assert.deepEqual(card.cannot, ["channel:admin", "token:admin"]);
});

test("the front door hint is read once and taken off the query", () => {
  assert.deepEqual(readFrontDoorHint("?choose_workspace=1"), { hint: "choose", rest: "" });
  assert.deepEqual(readFrontDoorHint("?no_workspace=1&x=2"), { hint: "no-workspace", rest: "?x=2" });
  assert.deepEqual(readFrontDoorHint("?choose_workspace=0"), { hint: null, rest: "" });
  assert.deepEqual(readFrontDoorHint(""), { hint: null, rest: "" });
});

test("a rotated secret is installed only while the connection is unchanged", async () => {
  const calls: string[] = [];
  const fetchFn = async (url: string) => {
    calls.push(url);
    if (url.endsWith("/auth/session/from-token")) return new Response("{}", { status: 200 });
    if (url.endsWith("/auth/session")) return new Response(JSON.stringify({ member_id: "m1", workspace_id: "w1", token_id: "t1" }), { status: 200 });
    return new Response(JSON.stringify({ member_id: "m1", workspace_id: "w1", is_bearer: true }), { status: 200 });
  };
  const s = new SessionStore("http://x", fetchFn);
  await s.signInWithToken("first");
  const stale = s.connection();
  s.signOut();
  assert.equal(await s.adoptRotated("rotated", stale), false);
  assert.equal(calls.filter((u) => u.endsWith("/from-token")).length, 1, "the stale rotation made no new session");
  await s.signInWithToken("again");
  assert.equal(await s.adoptRotated("rotated", s.connection()), true);
});
