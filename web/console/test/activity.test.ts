import { test } from "node:test";
import assert from "node:assert/strict";
import { filterActivity, groupOf, mergeActivity, toActivity, type ActivityRow } from "../src/core/activity.ts";
import { ROUTES, parseRoute } from "../src/shell/router.ts";

test("REST rows and live frames map to one shape", () => {
  assert.deepEqual(toActivity({ id: 5, kind: "thread_created", thread_id: "t1", channel_id: "c1", payload: { actor_id: "m1" }, occurred_at: "2026-11-01T00:00:00Z" }), {
    id: 5, kind: "thread_created", threadId: "t1", channelId: "c1", actorId: "m1", at: "2026-11-01T00:00:00Z",
  });
  assert.deepEqual(toActivity({ log_id: 6, kind: "vote_cast", thread_id: "t1", member_id: "m2" }), {
    id: 6, kind: "vote_cast", threadId: "t1", channelId: null, actorId: "m2", at: null,
  });
  assert.equal(toActivity({ kind: "x" }), null);
});

test("merge dedupes by id, newest first, capped", () => {
  const r = (id: number): ActivityRow => ({ id, kind: "k", threadId: null, channelId: null, actorId: null, at: null });
  assert.deepEqual(mergeActivity([r(1), r(3)], [r(3), r(2), r(4)]).map((x) => x.id), [4, 3, 2, 1]);
  assert.deepEqual(mergeActivity([r(1), r(2), r(3)], [], 2).map((x) => x.id), [3, 2]);
});

test("groups filter by kind family", () => {
  assert.equal(groupOf("vote_retracted"), "votes");
  assert.equal(groupOf("reaction_added"), "messages");
  assert.equal(groupOf("thread_state_changed"), "tasks");
  const rows = [toActivity({ id: 1, kind: "vote_cast" })!, toActivity({ id: 2, kind: "message_posted" })!];
  assert.deepEqual(filterActivity(rows, "votes").map((x) => x.id), [1]);
  assert.equal(filterActivity(rows, "all").length, 2);
});

test("activity is a route", () => {
  assert.ok(ROUTES.some((r) => r.id === "activity"));
  assert.equal(parseRoute("#/activity"), "activity");
});
