import assert from "node:assert/strict";
import { test } from "node:test";
import { LEASE_WARN_MS, boardHealth, healthMatch, type Thread } from "../src/core/board.ts";

const NOW = Date.parse("2026-11-01T12:00:00Z");
const th = (o: Partial<Thread> & { id: string }): Thread => ({ state: "open", ...o });
const at = (ms: number) => new Date(NOW + ms).toISOString();

test("the strip counts each live signal and skips zeros", () => {
  const h = boardHealth(
    [
      th({ id: "a", assignee_id: "m", stalled: true, assignment_expires_at: at(60_000) }),
      th({ id: "b", assignee_id: "m", stalled: true, assignment_expires_at: at(LEASE_WARN_MS * 3) }),
      th({ id: "c" }),
      th({ id: "d", state: "closed", stalled: true }),
    ],
    NOW,
  );
  assert.deepEqual(h.map((x) => [x.key, x.count, x.text]), [
    ["stalled", 2, "2 stalled"],
    ["ending", 1, "1 lease ending"],
    ["unclaimed", 1, "1 unclaimed"],
  ]);
  assert.deepEqual(boardHealth([th({ id: "x", assignee_id: "m" })], NOW), []);
});

test("a lapsed lease still counts as ending; no lease or no claim does not", () => {
  assert.equal(healthMatch(th({ id: "a", assignee_id: "m", assignment_expires_at: at(-5_000) }), "ending", NOW), true);
  assert.equal(healthMatch(th({ id: "a", assignee_id: "m" }), "ending", NOW), false);
  assert.equal(healthMatch(th({ id: "a", assignment_expires_at: at(1_000) }), "ending", NOW), false);
  assert.equal(healthMatch(th({ id: "a", assignee_id: "m", assignment_expires_at: "nonsense" }), "ending", NOW), false);
});

test("waiting covers waiting and overdue; in-review is never unclaimed; done never matches", () => {
  assert.equal(healthMatch(th({ id: "a", human_wait: { state: "overdue" } }), "waiting", NOW), true);
  assert.equal(healthMatch(th({ id: "a", human_wait: { state: "returned_to_queue" } }), "waiting", NOW), false);
  assert.equal(healthMatch(th({ id: "a", state: "in_review" }), "unclaimed", NOW), false);
  assert.equal(healthMatch(th({ id: "a", state: "archived", block: { reason: "x" } }), "blocked", NOW), false);
  assert.equal(healthMatch(th({ id: "a", block: { reason: "x" } }), "blocked", NOW), true);
});
