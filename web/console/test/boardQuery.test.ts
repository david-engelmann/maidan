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
  const h = writeQuery({ task: "t1", channel: "c1", agent: "m1", state: "review", q: "login fix" });
  assert.equal(h, "#/board?agent=m1&state=review&q=login+fix&channel=c1&task=t1");
  assert.deepEqual(readQuery(h), { task: "t1", channel: "c1", agent: "m1", state: "review", q: "login fix" });
  assert.equal(readQuery("#/board?state=nope").state, null);
  assert.equal(writeQuery({ task: null, channel: "c1", agent: null, state: null, q: "  " }), "#/board");
});

test("a card matches only when every filter set agrees", () => {
  const c = card({ id: "abc123", title: "Fix the Login race", lane: "working", who: "m1" });
  const none = { task: null, agent: null, state: null, q: "" };
  assert.ok(matches(c, none));
  assert.ok(matches(c, { ...none, q: "login" }));
  assert.ok(matches(c, { ...none, q: "abc" }));
  assert.ok(!matches(c, { ...none, q: "upload" }));
  assert.ok(matches(c, { ...none, agent: "m1", state: "working" }));
  assert.ok(!matches(c, { ...none, agent: "m2" }));
  assert.ok(!matches(c, { ...none, state: "open" }));
});

test("a task link names its channel, and a bare URL keeps a paren it opened", async () => {
  assert.match(writeQuery({ task: "t1", channel: "c1", agent: null, state: null, q: "" }), /channel=c1&task=t1/);
  const { parseInline } = await import("../src/core/richtext.ts");
  const link = parseInline("see https://example.org/wiki/Function_(mathematics) now").find((p) => p.kind === "link");
  assert.equal(link && "href" in link ? link.href : null, "https://example.org/wiki/Function_(mathematics)");
  const end = parseInline("(see https://example.org/a)").find((p) => p.kind === "link");
  assert.equal(end && "href" in end ? end.href : null, "https://example.org/a");
});
