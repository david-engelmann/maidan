import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import {
  BODY_REQUIRED,
  CHOOSE_WHO,
  GROUP_MIN,
  NOT_YOURSELF,
  ThreadsApi,
  ThreadsError,
  aggregateReactions,
  atBottom,
  convHash,
  dmConversation,
  dmOther,
  groupConversation,
  groupRefusal,
  keyboardInset,
  movesList,
  onNewMessages,
  parseConv,
  slashResult,
  startsGroup,
  touchesThread,
  visibleMessages,
  type Message,
} from "../src/core/threads.ts";
import { parseRoute } from "../src/shell/router.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };

type Call = { url: string; init?: RequestInit };
function fake(responses: Array<{ status?: number; body?: unknown }>, creds: Credentials = session) {
  const calls: Call[] = [];
  let i = 0;
  const api = new ThreadsApi(
    async (url, init) => {
      calls.push({ url, init });
      const r = responses[Math.min(i++, responses.length - 1)] ?? {};
      const text = r.body === undefined ? "" : JSON.stringify(r.body);
      return new Response(text, { status: r.status ?? 200 });
    },
    () => creds,
    () => "session",
  );
  return { api, calls };
}

const msg = (o: Partial<Message> & { id: string }): Message => ({ thread_id: "t", author_id: "a", body: "b", posted_at: "2026-10-30T10:00:00Z", ...o });

test("deep links round-trip and every one lands on the Threads route", () => {
  for (const ref of [{ kind: "task", id: "t1" }, { kind: "dm", id: "d1" }, { kind: "group", id: "g1" }] as const) {
    const h = convHash(ref);
    assert.deepEqual(parseConv(h), ref);
    assert.equal(parseRoute(h), "threads");
  }
  assert.equal(parseConv("#/threads"), null);
  assert.equal(parseConv("#/board/t/x"), null);
  assert.equal(parseConv("#/threads/zz/x"), null);
});

test("a DM names the other person; a group names itself and its members", () => {
  const members = new Map([["me", { id: "me", name: "David" }], ["b", { id: "b", name: "Builder", kind: "agent" }], ["c", { id: "c", name: "Cleo" }]]);
  const dm = { id: "d", member_low_id: "b", member_high_id: "me", thread_id: "td" };
  assert.equal(dmOther(dm, "me"), "b");
  assert.equal(dmOther({ ...dm, member_low_id: "me", member_high_id: "b" }, "me"), "b");
  const c = dmConversation(dm, "me", members);
  assert.equal(c.label, "Builder");
  assert.equal(c.detail, "Agent");
  assert.equal(c.threadId, "td");
  const g = groupConversation({ id: "g", thread_id: "tg", title: null, member_ids: ["me", "b", "c"] }, "me", members);
  assert.equal(g.label, "(untitled)");
  assert.equal(g.detail, "3 members · Builder, Cleo");
});

test("removed messages drop out and the rest read oldest first", () => {
  const out = visibleMessages([msg({ id: "2", posted_at: "2026-10-30T10:02:00Z" }), msg({ id: "x", tombstoned_at: "2026-10-30T10:03:00Z" }), msg({ id: "1" })]);
  assert.deepEqual(out.map((m) => m.id), ["1", "2"]);
});

test("one author's run within five minutes shares a header", () => {
  const a = msg({ id: "1" });
  assert.equal(startsGroup(undefined, a), true);
  assert.equal(startsGroup(a, msg({ id: "2", posted_at: "2026-10-30T10:04:00Z" })), false);
  assert.equal(startsGroup(a, msg({ id: "3", posted_at: "2026-10-30T10:06:00Z" })), true);
  assert.equal(startsGroup(a, msg({ id: "4", author_id: "z" })), true);
});

test("reactions count and know which are mine", () => {
  const r = aggregateReactions([{ member_id: "me", emoji: "👍" }, { member_id: "b", emoji: "👍" }, { member_id: "b", emoji: "🎉" }], "me");
  assert.deepEqual(r, [{ emoji: "👍", count: 2, mine: true }, { emoji: "🎉", count: 1, mine: false }]);
});

test("slash results read as the classic thread did", () => {
  assert.equal(slashResult({}), null);
  assert.deepEqual(slashResult({ slash_command: { name: "deploy", args: "prod" }, slash_response: { ok: true, response: "done" } }), { head: "/deploy prod", tone: "ok", status: "ok", response: "done" });
  assert.equal(slashResult({ slash_command: { name: "x" }, slash_response: { retrying: true, error: "busy" } })?.status, "retrying · busy");
  assert.equal(slashResult({ slash_command: { name: "x" }, slash_response: { error_kind: "trap", error: "boom", exit_code: 3 } })?.status, "trap · boom (exit 3)");
});

test("live frames: an open thread reloads only for its own content", () => {
  assert.equal(touchesThread({ kind: "message_posted", thread_id: "t" }, "t"), true);
  assert.equal(touchesThread({ kind: "reaction_added", thread_id: "t" }, "t"), true);
  assert.equal(touchesThread({ kind: "message_posted", thread_id: "u" }, "t"), false);
  assert.equal(touchesThread({ kind: "thread_state_changed", thread_id: "t" }, "t"), false);
  assert.equal(touchesThread({ kind: "message_posted", thread_id: "t" }, null), false);
  assert.equal(movesList("thread_created"), true);
  assert.equal(movesList("reaction_added"), false);
});

test("new messages: follow the end only when the reader is there", () => {
  assert.deepEqual(onNewMessages(true, false, ["1"], ["1", "2"]), { follow: true, unseen: 0 });
  assert.deepEqual(onNewMessages(false, false, ["1"], ["1", "2", "3"]), { follow: false, unseen: 2 });
  assert.deepEqual(onNewMessages(false, true, ["1"], ["1", "2"]), { follow: true, unseen: 0 });
  assert.deepEqual(onNewMessages(false, false, [], ["1"]), { follow: true, unseen: 0 });
  assert.deepEqual(onNewMessages(false, false, ["1", "2"], ["1", "2"]), { follow: false, unseen: 0 });
  assert.equal(atBottom({ scrollTop: 552, scrollHeight: 1000, clientHeight: 400 }), true);
  assert.equal(atBottom({ scrollTop: 300, scrollHeight: 1000, clientHeight: 400 }), false);
});

test("the keyboard inset comes from the visual viewport", () => {
  assert.equal(keyboardInset(844, null), 0);
  assert.equal(keyboardInset(844, { height: 844, offsetTop: 0 }), 0);
  assert.equal(keyboardInset(844, { height: 508, offsetTop: 0 }), 336);
  assert.equal(keyboardInset(844, { height: 508, offsetTop: 120 }), 216);
  assert.equal(keyboardInset(844, { height: 900, offsetTop: 0 }), 0);
});

test("posting goes to the route each kind owns, with an idempotency key", async () => {
  const { api, calls } = fake([{ body: { id: "m" } }]);
  await api.post({ kind: "task", id: "t" }, "  hi  ");
  await api.post({ kind: "dm", id: "d" }, "hi");
  await api.post({ kind: "group", id: "g" }, "hi");
  assert.deepEqual(calls.map((c) => c.url), ["http://h/ui/api/threads/t/messages", "http://h/ui/api/dm/d/messages", "http://h/ui/api/group-dms/g/messages"]);
  assert.equal(calls[0].init?.method, "POST");
  assert.equal(calls[0].init?.body, JSON.stringify({ body: "hi" }));
  assert.ok(new Headers(calls[0].init?.headers).get("Idempotency-Key"));
  await assert.rejects(api.post({ kind: "task", id: "t" }, "   "), new ThreadsError(BODY_REQUIRED));
  assert.equal(calls.length, 3);
});

test("a token reads from the bearer API, a session from /ui/api", async () => {
  const s = fake([{ body: [] }]);
  await s.api.messages("t");
  assert.equal(s.calls[0].url, "http://h/ui/api/threads/t/messages?limit=50");
  const b = fake([{ body: [] }], bearer);
  await b.api.dms("w", "me");
  assert.equal(b.calls[0].url, "http://h/workspaces/w/dm?member_id=me");
  assert.equal(new Headers(b.calls[0].init?.headers).get("Authorization"), "Bearer tok");
});

test("opening a DM or group refuses bad picks before the network", async () => {
  const { api, calls } = fake([{ body: { id: "d" } }]);
  await assert.rejects(api.openDm("w", "me", null), new ThreadsError(CHOOSE_WHO));
  await assert.rejects(api.openDm("w", "me", "me"), new ThreadsError(NOT_YOURSELF));
  assert.equal(groupRefusal("me", ["b"]), GROUP_MIN);
  assert.equal(groupRefusal("me", ["b", "me"]), GROUP_MIN);
  assert.equal(groupRefusal("me", ["b", "c"]), null);
  await assert.rejects(api.openGroupDm("w", "me", ["b"], ""), new ThreadsError(GROUP_MIN));
  assert.equal(calls.length, 0);
  await api.openGroupDm("w", "me", ["b", "c"], "  ");
  assert.equal(calls[0].url, "http://h/ui/api/workspaces/w/group-dms");
  assert.deepEqual(JSON.parse(String(calls[0].init?.body)), { member_ids: ["me", "b", "c"], title: null });
});

test("a refused write shows the server's own sentence; other failures stay plain", async () => {
  const r = fake([{ status: 422, body: { message: "body too long" } }]);
  await assert.rejects(r.api.post({ kind: "dm", id: "d" }, "x"), new ThreadsError("body too long"));
  const s = fake([{ status: 500, body: {} }]);
  await assert.rejects(s.api.messages("t"), (e: unknown) => e instanceof ThreadsError && !/body/.test((e as Error).message));
});

test("reactions toggle with POST and DELETE", async () => {
  const { api, calls } = fake([{ body: {} }]);
  await api.react("m", "👍", true);
  await api.react("m", "👍", false);
  assert.deepEqual(calls.map((c) => `${c.init?.method} ${c.url}`), ["POST http://h/ui/api/messages/m/reactions", "DELETE http://h/ui/api/messages/m/reactions"]);
});
