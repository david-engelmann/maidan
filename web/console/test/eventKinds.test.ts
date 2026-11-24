import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { test } from "node:test";
import { ALL_EVENT_KINDS, EVENT_GROUPS, chosenIn, eventLabel, inOrder, withGroup } from "../src/core/eventKinds.ts";

// The server's own list: the `EventKind` enum in the OpenAPI spec the console
// is generated from. The server refuses a webhook for no kind it knows
// (`parse_event_kinds` in crates/maidan-server/src/webhooks.rs has no deny
// list), so the console offers every one and leaves none out.
const spec = JSON.parse(readFileSync(resolve(import.meta.dirname, "../../../crates/maidan-server/tests/fixtures/wire/openapi.json"), "utf8"));
const server: string[] = spec.components.schemas.EventKind.enum;

test("the console offers exactly the kinds the server's spec lists, no more and no fewer", () => {
  const mine = new Set(ALL_EVENT_KINDS);
  assert.deepEqual(server.filter((k) => !mine.has(k)), [], "a kind the server has and the console does not offer");
  assert.deepEqual(ALL_EVENT_KINDS.filter((k) => !server.includes(k)), [], "a kind the console offers and the server does not have");
  assert.equal(ALL_EVENT_KINDS.length, server.length);
});

test("each kind is in one group only, and each group has a title and kinds", () => {
  assert.equal(new Set(ALL_EVENT_KINDS).size, ALL_EVENT_KINDS.length);
  assert.equal(new Set(EVENT_GROUPS.map((g) => g.id)).size, EVENT_GROUPS.length);
  for (const g of EVENT_GROUPS) {
    assert.ok(g.title.trim());
    assert.ok(g.kinds.length > 0, g.id);
  }
});

test("every kind has a short label a person can read", () => {
  for (const k of ALL_EVENT_KINDS) {
    const label = eventLabel(k);
    assert.match(label, /^[A-Z][a-z]/, k);
    assert.doesNotMatch(label, /_/, k);
    assert.ok(label.length <= 40, k);
  }
  assert.equal(eventLabel("thread_result_set"), "Thread result set");
  assert.equal(eventLabel("message_tombstoned"), "Message removed");
  assert.equal(eventLabel("something_new"), "Something new", "a kind from a newer server still reads");
});

test("choosing a group switches all of its kinds on or off and leaves the others", () => {
  const claims = EVENT_GROUPS.find((g) => g.id === "claims");
  assert.ok(claims);
  const on = withGroup(claims, ["message_posted"], true);
  assert.equal(chosenIn(claims, on), claims.kinds.length);
  assert.ok(on.includes("message_posted"));
  const off = withGroup(claims, on, false);
  assert.deepEqual(off, ["message_posted"]);
  assert.equal(chosenIn(claims, off), 0);
  assert.deepEqual(withGroup(claims, on, true).filter((k) => claims.kinds.includes(k)).length, claims.kinds.length, "turning a group on twice does not repeat a kind");
});

test("chosen kinds are sent in the groups' order", () => {
  assert.deepEqual(inOrder(["message_posted", "thread_result_set", "nope"]), ["thread_result_set", "message_posted"]);
});
