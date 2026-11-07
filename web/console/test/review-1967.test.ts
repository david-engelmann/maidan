import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import { BoardApi, LANE_PAGE, badgeTargets, limitForOpen, presenceStrip, presenceWords, shownBadgeTargets, toLanes, type Member, type PresenceWord, type Thread } from "../src/core/board.ts";
import { gatherFiles } from "../src/core/files.ts";
import { directThreadRef, mergeReads, parseConv, settleMembers, type Dm, type GroupDm } from "../src/core/threads.ts";

const token: Credentials = { base: "https://m.example/", pastedToken: "tok", tokenSession: false, sessionMemberId: "me" };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });

test("a DM thread id resolves to the conversation, and /d/ is a DM link", () => {
  const dms: Dm[] = [{ id: "dm-1", member_low_id: "a", member_high_id: "b", thread_id: "thread-dm" }];
  const groups: GroupDm[] = [{ id: "g-1", thread_id: "thread-g", member_ids: ["a", "b", "c"] }];
  assert.deepEqual(directThreadRef("thread-dm", dms, groups), { kind: "dm", id: "dm-1" });
  assert.deepEqual(directThreadRef("thread-g", dms, groups), { kind: "group", id: "g-1" });
  assert.equal(directThreadRef("task-1", dms, groups), null);
  assert.deepEqual(parseConv("#/threads/d/dm-1"), { kind: "dm", id: "dm-1" });
  assert.deepEqual(parseConv("#/threads/dm/dm-1"), { kind: "dm", id: "dm-1" });
  assert.deepEqual(parseConv("#/threads/t/task-1"), { kind: "task", id: "task-1" });
});

test("vote badges cover the rendered review and working window, not a cap of 40", () => {
  const th = (id: string, state: string): Thread => ({ id, state, title: id });
  const review = Array.from({ length: 70 }, (_, i) => th(`r${String(i).padStart(2, "0")}`, "in_review"));
  const working = Array.from({ length: 10 }, (_, i) => th(`w${i}`, "in_progress"));
  working.forEach((t) => {
    t.assignee_id = "a";
  });
  const lanes = toLanes([...review, ...working, th("o", "open")], {});
  const ids = badgeTargets(lanes, { review: 60, working: 60 });
  assert.equal(ids.length, 70);
  assert.equal(ids[0]?.startsWith("r"), true);
  assert.equal(ids.includes("r40"), true);
  assert.equal(ids.includes("r60"), false);
  assert.equal(ids.includes("w0"), true);
  assert.equal(ids.includes("o"), false);
  assert.equal(badgeTargets(lanes).length, 70);
  assert.equal(badgeTargets(lanes, 1).length, 1);
  const shown = [...lanes.review.slice(0, 60), ...lanes.working.slice(0, 60), ...lanes.open.slice(0, 60)];
  const agentIds = shownBadgeTargets(shown);
  assert.equal(agentIds.includes("o"), false);
  assert.equal(agentIds.length, 70);
});

test("the message fallback reads 500 and refuses a full page", async () => {
  const calls: string[] = [];
  const fetch = async (url: string) => {
    calls.push(url);
    if (url.endsWith("/threads/t/votes")) return json({ error: "missing" }, 404);
    if (url.includes("/messages?")) return json(Array.from({ length: 500 }, (_, i) => ({ id: `m${i}` })));
    return json([]);
  };
  const api = new BoardApi(fetch, () => token, () => "token");
  assert.equal(await api.cardVotes("t"), null);
  assert.match(calls[1] ?? "", /\/threads\/t\/messages\?limit=500$/);
  assert.equal(calls.length, 2);
});

test("a deep-linked card past the first page widens that lane to the next page", () => {
  const cards = Array.from({ length: 80 }, (_, i) => ({ thread: { id: `c${i}` } }));
  assert.equal(limitForOpen(cards, LANE_PAGE, "c64"), LANE_PAGE * 2);
  assert.equal(limitForOpen(cards, LANE_PAGE, "c59"), LANE_PAGE);
  assert.equal(limitForOpen(cards, LANE_PAGE, "c60"), LANE_PAGE * 2);
  assert.equal(limitForOpen(cards, LANE_PAGE, "missing"), LANE_PAGE);
  assert.equal(limitForOpen(cards, LANE_PAGE * 2, "c64"), LANE_PAGE * 2);
});

test("presence is empty while live updates are down, and an unloaded roster does not flash an id", () => {
  const members = new Map<string, Member>([
    ["agent", { id: "agent", name: "Zed", kind: "agent" }],
    ["human", { id: "human", name: "Mia", kind: "human" }],
  ]);
  const presence = new Map<string, PresenceWord>([
    ["aaa-unknown", "online"],
    ["human", "online"],
    ["agent", "online"],
    ["bbb-unknown", "away"],
  ]);
  const threads: Thread[] = [{ id: "t", state: "open", assignee_id: "agent" }];
  assert.deepEqual(presenceStrip(members, presence, threads, { kind: "reconnecting" }), []);
  assert.deepEqual(presenceStrip(members, presence, threads, { kind: "refused" }), []);
  assert.deepEqual(presenceStrip(members, presence, threads, { kind: "closed" }), []);
  const up = presenceStrip(members, presence, threads, { kind: "connected" });
  assert.deepEqual(up.map((p) => p.name), ["Zed", "Mia", "aaa-unkn", "bbb-unkn"]);
  assert.equal(up[0]?.holding, 1);
  const pending = presenceStrip(new Map(), new Map([["abcdef0123456789", "online"]]), []);
  assert.equal(pending[0]?.name, "");
  const settled = presenceStrip(settleMembers(new Map()), new Map([["abcdef0123456789", "online"]]), []);
  assert.equal(settled[0]?.name, "abcdef01");
});

test("a holding count names this channel", () => {
  assert.equal(presenceWords({ status: "online", holding: 0 }), "online, free");
  assert.equal(presenceWords({ status: "away", holding: 1 }), "away, holding 1 task in this channel");
  assert.equal(presenceWords({ status: "online", holding: 3 }), "online, holding 3 tasks in this channel");
});

test("a failed metadata read is not the same as a file with no optional fields", () => {
  const sha = "a".repeat(64);
  const other = "b".repeat(64);
  const rows = gatherFiles(
    [{ id: "t", title: "Notes" }],
    new Map([["t", [sha, other]]]),
    new Map([[sha, null], [other, { filename: "ok.txt" }]]),
    new Set([sha]),
  );
  assert.equal(rows.find((r) => r.sha === sha)?.metaFailed, true);
  assert.equal(rows.find((r) => r.sha === other)?.metaFailed, false);
  const quiet = gatherFiles([{ id: "t", title: "Notes" }], new Map([["t", [sha]]]), new Map([[sha, null]]));
  assert.equal(quiet[0]?.metaFailed, false);
});

test("a failed vote reload keeps the tally already shown", () => {
  const prev = { m1: [{ kind: "approve", count: 1, voters: ["a"], mine: false }] };
  const kept = mergeReads(prev, [{ id: "m1", rows: [], failed: true }], true);
  assert.equal(kept.stale, true);
  assert.equal(kept.next.m1?.[0]?.count, 1);
  const first = mergeReads({}, [{ id: "m1", rows: [], failed: true }], false);
  assert.equal(first.stale, false);
  assert.deepEqual(first.next.m1, []);
  const fresh = mergeReads(prev, [{ id: "m1", rows: [{ kind: "approve", count: 2, voters: ["a", "b"], mine: false }], failed: false }], true);
  assert.equal(fresh.stale, false);
  assert.equal(fresh.next.m1?.[0]?.count, 2);
});
