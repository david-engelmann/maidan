import assert from "node:assert/strict";
import { test } from "node:test";
import { BoardApi, BoardError, actionsFor, ago, cardAriaLabel, chips, chromeFor, closeFacts, leaseLeft, movesBoard, toLanes, type Thread } from "../src/core/board.ts";
import type { Credentials } from "../src/core/api.ts";
import { LiveBus } from "../src/core/bus.ts";

const th = (o: Partial<Thread> & { id: string }): Thread => ({ state: "open", ...o });
const NOW = Date.parse("2026-10-30T12:00:00Z");

test("chrome follows the classic board's order", () => {
  assert.equal(chromeFor(th({ id: "a", state: "closed" }), { hasSchema: true }).lane, "done");
  assert.equal(chromeFor(th({ id: "a", assignee_id: "m" }), { hasSchema: true }).label, "needs-input");
  assert.equal(chromeFor(th({ id: "a" }), { hasSchema: false }).label, "needs-approval");
  assert.equal(chromeFor(th({ id: "a", state: "in_review", assignee_id: "m", work_started_at: "x" })).label, "in review");
  assert.deepEqual(chromeFor(th({ id: "a", assignee_id: "m", work_started_at: "x" })).lane, "working");
  assert.equal(chromeFor(th({ id: "a", assignee_id: "m" })).label, "claimed");
  assert.equal(chromeFor(th({ id: "a" })).lane, "open");
});

test("lanes hold every task, newest change first, untitled by short id", () => {
  const l = toLanes(
    [th({ id: "11111111aaaa", updated_at: "2026-10-30T10:00:00Z" }), th({ id: "b", title: "B", updated_at: "2026-10-30T11:00:00Z" }), th({ id: "c", state: "archived" })],
    {},
  );
  assert.deepEqual(l.open.map((c) => c.title), ["B", "11111111"]);
  assert.equal(l.done.length, 1);
  assert.equal(toLanes([th({ id: "d", state: "closed", closed_without_review: true })], {}).done[0].label, "closed without review");
});

test("close facts say completed by after review, closed by without (#1792)", () => {
  const rows = [
    { actor_id: "agent", to_state: "in_review", at: "t1" },
    { actor_id: "dave", to_state: "closed", at: "t2" },
  ];
  assert.deepEqual(closeFacts(th({ id: "a", state: "closed" }), rows), { verb: "completed by", actorId: "dave", at: "t2", handedOffBy: "agent" });
  const skip = closeFacts(th({ id: "a", state: "closed", closed_without_review: true }), [{ actor_id: "dave", to_state: "closed", created_at: "t3" }]);
  assert.deepEqual(skip, { verb: "closed by", actorId: "dave", at: "t3", handedOffBy: null });
  assert.equal(closeFacts(th({ id: "a", state: "closed" }), [{ actor_id: "x", to_state: "in_review" }, { actor_id: "x", to_state: "closed" }])?.handedOffBy, null);
  assert.equal(closeFacts(th({ id: "a" }), []), null);
});

test("actions: claim needs a token, close needs review or my claim", () => {
  assert.deepEqual(actionsFor(th({ id: "a" }), "me", true), { claim: null, close: false });
  assert.match(String(actionsFor(th({ id: "a" }), "me", false).claim), /needs a token/);
  assert.deepEqual(actionsFor(th({ id: "a", assignee_id: "me" }), "me", true), { claim: false, close: true });
  assert.equal(actionsFor(th({ id: "a", assignee_id: "you" }), "me", true).close, false);
  assert.equal(actionsFor(th({ id: "a", state: "in_review", assignee_id: "you" }), "me", false).close, true);
  assert.deepEqual(actionsFor(th({ id: "a", state: "closed" }), "me", true), { claim: false, close: false });
});

test("chips and the card's accessible name", () => {
  const t = th({ id: "a", title: "T", stalled: true, block: { reason: "dependency" }, human_wait: { state: "overdue" } });
  assert.deepEqual(chips(t, "open").map((c) => c.kind), ["stalled", "wait-overdue", "blocked", "unclaimed"]);
  assert.deepEqual(chips(th({ id: "a", state: "closed" }), "done"), []);
  const card = toLanes([t], {}).open[0];
  assert.equal(cardAriaLabel(card), "T: open, stalled, overdue on a person");
});

test("times read as people say them", () => {
  assert.equal(ago("2026-10-30T11:59:50Z", NOW), "just now");
  assert.equal(ago("2026-10-30T11:55:00Z", NOW), "5m ago");
  assert.equal(ago("2026-10-30T09:00:00Z", NOW), "3h ago");
  assert.equal(ago("bad", NOW), "");
  assert.equal(leaseLeft("2026-10-30T12:00:30Z", NOW), "lease 30s");
  assert.equal(leaseLeft("2026-10-30T12:10:00Z", NOW), "lease 10m");
  assert.equal(leaseLeft("2026-10-30T11:00:00Z", NOW), "lease lapsed");
});

test("only board kinds move the board", () => {
  assert.ok(movesBoard("thread_state_changed"));
  assert.ok(!movesBoard("message_posted"));
  assert.ok(!movesBoard(undefined));
});

test("the bus fans frames out and stops on unlisten", () => {
  const bus = new LiveBus();
  const got: string[] = [];
  const off = bus.listen((m) => got.push(m.type));
  bus.emit({ type: "resync" });
  off();
  bus.emit({ type: "resync" });
  assert.deepEqual(got, ["resync"]);
});

const session: Credentials = { base: "https://m.example/", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const token: Credentials = { ...session, pastedToken: "tok" };
type Call = { url: string; init?: RequestInit };
function fake(reply: (url: string, init?: RequestInit) => Response) {
  const calls: Call[] = [];
  return { calls, fetch: async (url: string, init?: RequestInit) => (calls.push({ url, init }), reply(url, init)) };
}
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

test("threads page until a short page, by the last id", async () => {
  const f = fake((url) => {
    const cur = new URL(url).searchParams.get("cursor");
    return json(cur === null ? [{ id: "1" }, { id: "2" }] : cur === "2" ? [{ id: "3" }] : []);
  });
  const api = new BoardApi(f.fetch, () => session, () => "session");
  assert.deepEqual((await api.threads("c", 2)).map((t) => t.id), ["1", "2", "3"]);
  assert.equal(f.calls[0].url, "https://m.example/ui/api/channels/c/threads?limit=2");
  assert.equal(f.calls[1].url, "https://m.example/ui/api/channels/c/threads?limit=2&cursor=2");
});

test("a stuck cursor stops paging", async () => {
  const f = fake(() => json([{ id: "x" }]));
  const api = new BoardApi(f.fetch, () => session, () => "session");
  assert.equal((await api.threads("c", 1)).length, 2);
  assert.equal(f.calls.length, 2);
});

test("close posts the snake_case action through the session proxy", async () => {
  const f = fake(() => json({ id: "t", state: "closed" }));
  await new BoardApi(f.fetch, () => session, () => "session").close("t");
  assert.equal(f.calls[0].url, "https://m.example/ui/api/threads/t");
  assert.equal(f.calls[0].init?.method, "POST");
  assert.equal(f.calls[0].init?.body, '{"action":"close"}');
  assert.ok(new Headers(f.calls[0].init?.headers).get("Idempotency-Key"));
});

test("claim goes to the bearer route with an empty body", async () => {
  const f = fake(() => new Response(null, { status: 204 }));
  await new BoardApi(f.fetch, () => token, () => "bearer").claim("t");
  assert.equal(f.calls[0].url, "https://m.example/threads/t/assignee/claim");
  assert.equal(f.calls[0].init?.body, "{}");
  assert.equal(new Headers(f.calls[0].init?.headers).get("Authorization"), "Bearer tok");
});

test("a refused close shows the server's sentence; other errors stay human", async () => {
  const refused = new BoardApi(fake(() => json({ message: "a review is still open" }, 409)).fetch, () => session, () => "session");
  await assert.rejects(refused.close("t"), (e: unknown) => e instanceof BoardError && e.message === "Refused: a review is still open");
  const broken = new BoardApi(fake(() => json({ message: "stack trace" }, 500)).fetch, () => session, () => "session");
  await assert.rejects(broken.close("t"), (e: unknown) => e instanceof BoardError && !e.message.includes("stack trace"));
});

test("gates are best effort and keyed by thread", async () => {
  const ok = new BoardApi(fake(() => json([{ gate: { thread_id: "t", schema: {} } }, { gate: { thread_id: "u", schema: null } }])).fetch, () => session, () => "session");
  assert.deepEqual(await ok.gates("w"), { t: { hasSchema: true }, u: { hasSchema: false } });
  const down = new BoardApi(fake(() => json({}, 503)).fetch, () => session, () => "session");
  assert.deepEqual(await down.gates("w"), {});
});
