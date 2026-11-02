import assert from "node:assert/strict";
import { test } from "node:test";
import { matches, readQuery, writeQuery } from "../src/core/boardQuery.ts";
import type { Card } from "../src/core/board.ts";

const card = (o: { id: string; title: string; lane: Card["chrome"]["lane"]; who?: string }): Card => ({
  thread: { id: o.id, state: "open", assignee_id: o.who ?? null },
  chrome: { label: o.lane, hint: "", lane: o.lane },
  title: o.title,
  label: o.lane,
});

test("the board query round-trips through the hash and drops unknown states", () => {
  const h = writeQuery({ channel: null, task: "t1", agent: "m1", state: "review", q: "login fix" });
  assert.equal(h, "#/board?agent=m1&state=review&q=login+fix&task=t1");
  assert.deepEqual(readQuery(h), { channel: null, task: "t1", agent: "m1", state: "review", q: "login fix" });
  assert.equal(readQuery("#/board?state=nope").state, null);
  assert.equal(writeQuery({ channel: null, task: null, agent: null, state: null, q: "  " }), "#/board");
});

test("a card matches only when every filter set agrees", () => {
  const c = card({ id: "abc123", title: "Fix the Login race", lane: "working", who: "m1" });
  const none = { channel: null, task: null, agent: null, state: null, q: "" };
  assert.ok(matches(c, none));
  assert.ok(matches(c, { ...none, q: "login" }));
  assert.ok(matches(c, { ...none, q: "abc" }));
  assert.ok(!matches(c, { ...none, q: "upload" }));
  assert.ok(matches(c, { ...none, agent: "m1", state: "working" }));
  assert.ok(!matches(c, { ...none, agent: "m2" }));
  assert.ok(!matches(c, { ...none, state: "open" }));
});

test("board query: a task link carries its channel", () => {
  const h = writeQuery({ channel: "c9", task: "t1", agent: null, state: null, q: "" });
  assert.equal(h, "#/board?channel=c9&task=t1");
  assert.equal(readQuery(h).channel, "c9");
  assert.equal(readQuery("#/board?task=t1").channel, null);
});
