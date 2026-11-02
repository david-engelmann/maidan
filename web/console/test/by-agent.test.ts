import assert from "node:assert/strict";
import { test } from "node:test";
import { byAgent, type Member, type Thread } from "../src/core/board.ts";

const th = (o: Partial<Thread> & { id: string }): Thread => ({ state: "open", ...o });
const members = new Map<string, Member>([["m1", { id: "m1", name: "zed" }], ["m2", { id: "m2", name: "amy" }]]);

test("groups live tasks by holder A to Z, unclaimed last, done left out", () => {
  const g = byAgent(
    [
      th({ id: "a", assignee_id: "m1", work_started_at: "x" }),
      th({ id: "b", assignee_id: "m2" }),
      th({ id: "c" }),
      th({ id: "d", assignee_id: "m1", state: "in_review" }),
      th({ id: "e", assignee_id: "m1", state: "closed" }),
      th({ id: "f", assignee_id: "abcdef123456" }),
    ],
    {},
    members,
  );
  assert.deepEqual(g.map((x) => [x.name, x.cards.map((c) => c.thread.id)]), [
    ["abcdef12", ["f"]],
    ["amy", ["b"]],
    ["zed", ["d", "a"]],
    ["Unclaimed", ["c"]],
  ]);
});

test("an empty board has no groups", () => {
  assert.deepEqual(byAgent([], {}, members), []);
});
