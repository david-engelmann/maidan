import { test } from "node:test";
import assert from "node:assert/strict";
import { NEW_WORK_HREF, emptyChannelText, liveWord, shellTitle } from "../src/core/firstRun.ts";
import { parseRoute } from "../src/shell/router.ts";

test("a workspace with no channel says so and where to make one", () => {
  assert.equal(emptyChannelText(0), "No channels yet. Create one under Tools, then Workspace.");
  assert.equal(emptyChannelText(2), "No tasks in this channel yet.");
});

test("the start-a-task link lands on the screen that creates tasks", () => {
  assert.equal(parseRoute(NEW_WORK_HREF), "tools");
  assert.equal(NEW_WORK_HREF.split("/").pop(), "workspace");
});

test("signed out, the top bar says sign in, not a place or offline", () => {
  assert.equal(shellTitle(true, "Board"), "Sign in");
  assert.equal(shellTitle(false, "Board"), "Board");
  assert.equal(liveWord("offline", true), "Signed out");
  assert.equal(liveWord("connected", true), "Signed out");
  assert.equal(liveWord("connected", false), "Live");
  assert.equal(liveWord("reconnecting", false), "Reconnecting");
  assert.equal(liveWord("refused", false), "Refused");
  assert.equal(liveWord("offline", false), "Offline");
});
