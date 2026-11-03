import assert from "node:assert/strict";
import { test } from "node:test";
import { BoardApi, badgeFromRows, badgeFromThread, badgeTargets, hasVotes, toLanes, voteWords, type Thread } from "../src/core/board.ts";
import type { Credentials } from "../src/core/api.ts";

const token: Credentials = { base: "https://m.example/", pastedToken: "tok", tokenSession: false, sessionMemberId: "me" };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
function fake(reply: (url: string) => Response) {
  const calls: string[] = [];
  return { calls, fetch: async (url: string) => (calls.push(url), reply(url)) };
}

test("one call to the thread read sums every message", async () => {
  const f = fake(() => json({ thread_id: "t", messages: [{ message_id: "a", approve: 2, request_changes: 0, ack: 1 }, { message_id: "b", approve: 0, request_changes: 1, ack: 0 }] }));
  const api = new BoardApi(f.fetch, () => token, () => "token");
  assert.deepEqual(await api.cardVotes("t"), { approve: 2, request_changes: 1, ack: 1 });
  assert.deepEqual(f.calls, ["https://m.example/threads/t/votes"]);
});

test("a 404 falls back to per-message votes, and is remembered", async () => {
  const f = fake((url) => {
    if (url.endsWith("/votes") && url.includes("/threads/")) return json({ error: "not found" }, 404);
    if (url.includes("/threads/t/messages")) return json([{ id: "m1" }, { id: "m2" }]);
    if (url.endsWith("/messages/m1/votes")) return json([{ kind: "approve" }, { kind: "ack" }]);
    return json([{ kind: "request_changes" }]);
  });
  const api = new BoardApi(f.fetch, () => token, () => "token");
  assert.deepEqual(await api.cardVotes("t"), { approve: 1, request_changes: 1, ack: 1 });
  const before = f.calls.length;
  await api.cardVotes("t");
  assert.ok(!f.calls.slice(before).some((u) => u.endsWith("/threads/t/votes")), "the missing route is not asked again");
});

test("no vote read at all gives no badge", async () => {
  const f = fake(() => json({}, 404));
  assert.equal(await new BoardApi(f.fetch, () => token, () => "token").cardVotes("t"), null);
});

test("badge helpers", () => {
  assert.deepEqual(badgeFromRows([{ kind: "approve" }, { kind: "up" }]), { approve: 1, request_changes: 0, ack: 0 });
  assert.deepEqual(badgeFromThread({ thread_id: "t", messages: [] }), { approve: 0, request_changes: 0, ack: 0 });
  assert.equal(hasVotes({ approve: 0, request_changes: 0, ack: 0 }), false);
  assert.equal(voteWords({ approve: 2, request_changes: 1, ack: 0 }), "2 approve, 1 changes requested");
});

test("review cards are badged first, capped", () => {
  const th = (id: string, state: string) => ({ id, state, title: id, created_at: "2026-11-01T00:00:00Z" }) as unknown as Thread;
  const lanes = toLanes([th("o", "open"), th("r", "in_review"), th("w", "in_progress")], {});
  const ids = badgeTargets(lanes);
  assert.equal(ids[0], "r");
  assert.ok(!ids.includes("o"));
  assert.equal(badgeTargets(lanes, 1).length, 1);
});
