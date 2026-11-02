import assert from "node:assert/strict";
import { test } from "node:test";
import { groupConversation, nameOf } from "../src/core/threads.ts";

const id = "0f5c2a9e-1111-4222-8333-944455556666";

test("a name is blank until the member list loads, so no raw id flashes", () => {
  assert.equal(nameOf(new Map(), id), "");
  const g = { id: "g", thread_id: "t", title: "Crew", member_ids: ["me", id], updated_at: null };
  assert.equal(groupConversation(g as never, "me", new Map()).detail, "2 members");
});

test("once loaded, a known member reads by name and an unknown one by short id", () => {
  const members = new Map([[id, { id, name: "Rae", kind: "human" }]]) as never;
  assert.equal(nameOf(members, id), "Rae");
  assert.equal(nameOf(members, "abcdef0123456789"), "abcdef01");
});
