import assert from "node:assert/strict";
import { test } from "node:test";
import { presenceStrip, type Member, type PresenceWord, type Thread } from "../src/core/board.ts";

const members = new Map<string, Member>([
  ["a", { id: "a", name: "bot-b", kind: "agent" }],
  ["b", { id: "b", name: "bot-a", kind: "agent" }],
  ["c", { id: "c", name: "ann", kind: "human" }],
  ["d", { id: "d", name: "dan", kind: "human" }],
]);

test("online before away, agents before people, offline left out, holdings counted", () => {
  const presence = new Map<string, PresenceWord>([["a", "online"], ["b", "away"], ["c", "online"], ["d", "offline"], ["zzzzzzzz99", "online"]]);
  const threads: Thread[] = [
    { id: "t1", state: "open", assignee_id: "a" },
    { id: "t2", state: "in_review", assignee_id: "a" },
    { id: "t3", state: "closed", assignee_id: "c" },
  ];
  const s = presenceStrip(members, presence, threads);
  assert.deepEqual(s.map((p) => [p.name, p.status, p.holding]), [
    ["bot-b", "online", 2],
    ["ann", "online", 0],
    ["zzzzzzzz", "online", 0],
    ["bot-a", "away", 0],
  ]);
});
