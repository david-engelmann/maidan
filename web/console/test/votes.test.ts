import { test } from "node:test";
import assert from "node:assert/strict";
import { tallyVotes, touchesThread, ThreadsApi } from "../src/core/threads.ts";

test("tallyVotes groups by kind, approve first, voters in vote order", () => {
  const t = tallyVotes(
    [
      { member_id: "b", kind: "request_changes", created_at: "2026-11-01T02:00:00Z" },
      { member_id: "c", kind: "approve", created_at: "2026-11-01T03:00:00Z" },
      { member_id: "a", kind: "approve", created_at: "2026-11-01T01:00:00Z" },
      { member_id: "a", kind: "ack", created_at: "2026-11-01T04:00:00Z" },
    ],
    "c",
  );
  assert.deepEqual(
    t.map((v) => [v.kind, v.count, v.voters, v.mine]),
    [
      ["approve", 2, ["a", "c"], true],
      ["request_changes", 1, ["b"], false],
      ["ack", 1, ["a"], false],
    ],
  );
});

test("tallyVotes of nothing is empty; unknown kinds sort last", () => {
  assert.deepEqual(tallyVotes([], null), []);
  const t = tallyVotes([{ member_id: "x", kind: "zz" }, { member_id: "y", kind: "ack" }], null);
  assert.deepEqual(t.map((v) => v.kind), ["ack", "zz"]);
});

test("vote frames refresh the open conversation only", () => {
  assert.equal(touchesThread({ kind: "vote_cast", thread_id: "t1" }, "t1"), true);
  assert.equal(touchesThread({ kind: "vote_retracted", thread_id: "t1" }, "t1"), true);
  assert.equal(touchesThread({ kind: "vote_cast", thread_id: "t2" }, "t1"), false);
});
