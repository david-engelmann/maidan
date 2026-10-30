import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import { NeedsApi, parseConfirmLink } from "../src/core/needs.ts";
import { EDIT_UNCHANGED, ThreadsApi, editRefusal, evidenceLine, pinnedMessages, type Message } from "../src/core/threads.ts";
import { HANDLE_REQUIRED, ToolsApi, handleRefusal, peerRefusal, searchQuery, switchHref } from "../src/core/tools.ts";
import { parseRoute } from "../src/shell/router.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };
type Call = { url: string; init?: RequestInit };
function fetcher(responses: Array<{ status?: number; body?: unknown }>) {
  const calls: Call[] = [];
  let i = 0;
  const fn = async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const r = responses[Math.min(i++, responses.length - 1)] ?? {};
    return new Response(r.body === undefined ? "" : JSON.stringify(r.body), { status: r.status ?? 200 });
  };
  return { fn, calls };
}
const hdr = (c: Call, k: string) => new Headers(c.init?.headers).get(k);
const msg = (o: Partial<Message> & { id: string }): Message => ({ thread_id: "t", author_id: "a", body: "b", posted_at: "2026-10-30T10:00:00Z", ...o });

test("pinned messages keep thread order and drop deleted ones", () => {
  const rows = [msg({ id: "1" }), msg({ id: "2", tombstoned_at: "x" }), msg({ id: "3" })];
  assert.deepEqual(pinnedMessages(rows, new Set(["3", "2", "1"])).map((m) => m.id), ["1", "3"]);
});

test("an edit must say something new", () => {
  assert.ok(editRefusal("hi", "  "));
  assert.equal(editRefusal("hi", " hi "), EDIT_UNCHANGED);
  assert.equal(editRefusal("hi", "hello"), null);
});

test("pins, edits and history hit the thread and message routes", async () => {
  const f = fetcher([{ body: [{ message_id: "m1" }, {}] }, {}, {}, { body: { id: "m1", body: "new" } }, { body: [] }]);
  const api = new ThreadsApi(f.fn, () => session, () => "session");
  assert.deepEqual(await api.pins("t1"), ["m1"]);
  await api.pin("t1", "m1", true);
  await api.pin("t1", "m1", false);
  await api.edit({ id: "m1", body: "old" }, " new ");
  await api.edits("m1");
  assert.equal(f.calls[0].url, "http://h/ui/api/threads/t1/pins");
  assert.equal(f.calls[1].init?.method, "POST");
  assert.equal(f.calls[2].init?.method, "DELETE");
  assert.equal(f.calls[2].init?.body, JSON.stringify({ message_id: "m1" }));
  assert.equal(f.calls[3].init?.method, "PATCH");
  assert.equal(f.calls[3].init?.body, JSON.stringify({ body: "new" }));
  assert.match(f.calls[4].url, /\/messages\/m1\/edits/);
  await assert.rejects(api.edit({ id: "m1", body: "same" }, "same"), new RegExp(EDIT_UNCHANGED));
  assert.equal(f.calls.length, 5);
});

test("a confirm link opens Needs you and confirms on the session only", async () => {
  assert.equal(parseRoute("#confirm-approval=g1.abc"), "needs");
  assert.deepEqual(parseConfirmLink("#confirm-approval=g1.ab.c"), { gateId: "g1", token: "ab.c" });
  assert.equal(parseConfirmLink("#/needs"), null);
  const f = fetcher([{ body: {} }]);
  const api = new NeedsApi(f.fn, () => bearer, () => "bearer");
  await api.confirmLink({ gateId: "g1", token: "sekret" });
  const c = f.calls[0];
  assert.equal(c.url, "http://h/auth/approval-confirmations/confirm");
  assert.equal(hdr(c, "Authorization"), null);
  assert.ok(hdr(c, "Idempotency-Key"));
  assert.equal(c.init?.credentials, "include");
  assert.equal(c.init?.body, JSON.stringify({ gate_id: "g1", token: "sekret" }));
  const bad = new NeedsApi(fetcher([{ status: 403, body: { error: "expired" } }]).fn, () => session, () => "session");
  await assert.rejects(bad.confirmLink({ gateId: "g1", token: "x" }));
});

test("workspace writes use the documented bodies", async () => {
  const f = fetcher([{}, {}, {}, {}, {}, {}]);
  const api = new ToolsApi(f.fn, () => bearer, () => "bearer");
  await api.renameWorkspace("w", " New ");
  await api.createChannel("w", " ops ");
  await api.createThread("c", " Fix it ");
  await api.createAgent("w", "bot-1", "");
  await api.addPeer("w", "east", "https://e.example");
  await api.removePeer("w", "p1");
  const got = f.calls.map((c) => `${c.init?.method} ${c.url} ${c.init?.body ?? ""}`);
  assert.deepEqual(got, [
    'PATCH http://h/workspaces/w {"name":"New"}',
    'POST http://h/workspaces/w/channels {"name":"ops","private":false}',
    'POST http://h/channels/c/threads {"title":"Fix it"}',
    'POST http://h/workspaces/w/agents {"handle":"bot-1","display_name":null}',
    'POST http://h/workspaces/w/peers {"name":"east","base_url":"https://e.example"}',
    "DELETE http://h/workspaces/w/peers/p1 ",
  ]);
});

test("agent and peer writes refuse without a pasted token", async () => {
  const f = fetcher([{}]);
  const api = new ToolsApi(f.fn, () => session, () => "session");
  await assert.rejects(api.createAgent("w", "bot", ""));
  await assert.rejects(api.addPeer("w", "e", "https://e"));
  assert.equal(f.calls.length, 0);
});

test("search, audit and installs read the workspace", async () => {
  const f = fetcher([{ body: [] }, { body: [] }, { body: [] }]);
  const api = new ToolsApi(f.fn, () => session, () => "session");
  await api.search("w", searchQuery(" hi ", "lexical", "", " a1 "));
  await api.workspaceAudit("w", 100);
  await api.appInstallations("w");
  assert.equal(f.calls[0].url, "http://h/ui/api/workspaces/w/search?q=hi&mode=lexical&limit=20&author=a1");
  assert.equal(f.calls[1].url, "http://h/ui/api/workspaces/w/audit?limit=100");
  assert.equal(f.calls[2].url, "http://h/ui/api/workspaces/w/app-installations");
});

test("workspace switcher degrades to nothing and links back to the console", async () => {
  const ok = new ToolsApi(fetcher([{ body: { workspaces: [{ workspace_id: "a", name: "A", current: true }] } }]).fn, () => session, () => "session");
  assert.equal((await ok.switchable()).length, 1);
  const no = new ToolsApi(fetcher([{ status: 404 }]).fn, () => session, () => "session");
  assert.deepEqual(await no.switchable(), []);
  assert.equal(switchHref("http://h/", "w 1"), "http://h/auth/oidc/login?workspace_id=w%201&return_to=/console/");
});

test("handles and peers are checked before sending", () => {
  assert.equal(handleRefusal("Bad Handle"), HANDLE_REQUIRED);
  assert.equal(handleRefusal("bot-2"), null);
  assert.ok(peerRefusal("", "https://x"));
  assert.ok(peerRefusal("e", "ftp://x"));
  assert.equal(peerRefusal("e", "https://x"), null);
});

test("evidenceLine words each evidence kind like the old timeline", () => {
  const n = (id: string | undefined) => (id === "m1" ? "Ada" : "someone");
  assert.equal(evidenceLine({ kind: "artifact_linked", at: "", artifact: { sha256: "a".repeat(64), linked_by: "m1" } }, n), `Artifact ${"a".repeat(12)}… linked by Ada`);
  assert.equal(evidenceLine({ kind: "land_gate_verdict", at: "", verdict: { status: "pass" } }, n), "Land gate: pass");
  assert.equal(evidenceLine({ kind: "review_verdict", at: "", verdict: { decision: "changes_requested", reviewer_id: "m1", note: "fix" } }, n), "Review: changes requested by Ada: fix");
  assert.equal(evidenceLine({ kind: "weird", at: "" }, n), "weird");
});
