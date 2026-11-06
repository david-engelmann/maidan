import assert from "node:assert/strict";
import { test } from "node:test";
import { needsDataOnRoomChange, needsRoomKey } from "../src/hooks/useNeeds.ts";
import { paletteRefreshesOn } from "../src/shell/paletteIndex.ts";

test("signing out or switching room drops the previous waiting list", () => {
  const ada = { status: "signed-in", memberId: "ada", workspaceId: "room-a" };
  const bea = { status: "signed-in", memberId: "bea", workspaceId: "room-a" };
  const other = { status: "signed-in", memberId: "ada", workspaceId: "room-b" };
  const out = { status: "signed-out", memberId: null, workspaceId: null };
  assert.equal(needsRoomKey(ada), "room-a:ada");
  assert.notEqual(needsRoomKey(ada), needsRoomKey(bea));
  assert.notEqual(needsRoomKey(ada), needsRoomKey(other));
  assert.equal(needsRoomKey(out), null);
  assert.equal(needsRoomKey({ status: "loading", memberId: null, workspaceId: null }), null);
  const cleared = needsDataOnRoomChange(null);
  assert.equal(cleared.status, "idle");
  assert.deepEqual(cleared.items, []);
  assert.equal(cleared.staleSince, null);
  const loading = needsDataOnRoomChange(needsRoomKey(other));
  assert.equal(loading.status, "loading");
  assert.deepEqual(loading.items, []);
  assert.equal(loading.gates.size, 0);
  assert.notEqual(cleared.gates, loading.gates);
});

test("the palette refreshes when a channel, member, or task changes", () => {
  for (const kind of ["channel_created", "channel_updated", "member_joined", "member_updated", "thread_created", "thread_updated"]) {
    assert.equal(paletteRefreshesOn(kind), true, kind);
  }
  assert.equal(paletteRefreshesOn("message_posted"), false);
  assert.equal(paletteRefreshesOn("vote_cast"), false);
  assert.equal(paletteRefreshesOn(undefined), false);
  assert.equal(paletteRefreshesOn(""), false);
});
