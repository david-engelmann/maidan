import assert from "node:assert/strict";
import { test } from "node:test";
import { SELECT_CHANNEL_EVENT, channelJump, followChannelJump, matches, pickChannel, readQuery, writeQuery } from "../src/core/boardQuery.ts";
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

test("a channel link without a task still names the channel", () => {
  assert.equal(readQuery("#/board?channel=c1").channel, "c1");
  assert.equal(readQuery("#/board?channel=c1").task, null);
  assert.deepEqual(channelJump("needs", "#/needs", "c1"), { href: "#/board?channel=c1", select: false });
  assert.deepEqual(channelJump("threads", "#/threads/t/abc", "c1"), { href: null, select: true });
  assert.deepEqual(channelJump("board", "#/board?channel=c1", "c1"), { href: null, select: true });
  assert.deepEqual(channelJump("board", "#/board", "ops/1"), { href: "#/board?channel=ops%2F1", select: false });
});

test("the first read prefers the link's channel over the remembered one", () => {
  const channels = [{ id: "a" }, { id: "b" }];
  assert.equal(pickChannel(null, channels, "a", "b"), "b");
  assert.equal(pickChannel(null, channels, "a", null), "a");
  assert.equal(pickChannel("b", channels, "a", "a"), "b", "a menu choice wins over the link");
  assert.equal(pickChannel(null, channels, "missing", "also-missing"), "a");
  assert.equal(pickChannel(null, [], null, null), null);
});

test("a channel jump writes the board link, and on Threads it only selects", () => {
  const events: Event[] = [];
  const previous = (globalThis as { window?: unknown }).window;
  const location = { hash: "#/needs" };
  (globalThis as { window?: unknown }).window = { location, dispatchEvent: (e: Event) => events.push(e) };
  try {
    followChannelJump("needs", "c1");
    assert.equal(location.hash, "#/board?channel=c1");
    assert.equal(events.length, 0);
    location.hash = "#/threads";
    followChannelJump("threads", "c1");
    assert.equal(location.hash, "#/threads");
    assert.equal(events.length, 1);
    assert.equal(events[0].type, SELECT_CHANNEL_EVENT);
    assert.equal((events[0] as CustomEvent<string>).detail, "c1");
  } finally {
    (globalThis as { window?: unknown }).window = previous;
  }
});

test("a task link names its channel, and a bare URL keeps a paren it opened", async () => {
  assert.match(writeQuery({ task: "t1", channel: "c1", agent: null, state: null, q: "" }), /channel=c1&task=t1/);
  const { parseInline } = await import("../src/core/richtext.ts");
  const link = parseInline("see https://example.org/wiki/Function_(mathematics) now").find((p) => p.kind === "link");
  assert.equal(link && "href" in link ? link.href : null, "https://example.org/wiki/Function_(mathematics)");
  const end = parseInline("(see https://example.org/a)").find((p) => p.kind === "link");
  assert.equal(end && "href" in end ? end.href : null, "https://example.org/a");
});
