import { test } from "node:test";
import assert from "node:assert/strict";
import { NEW_WORK_HREF, dismissFirstRun, emptyChannelText, firstRunDismissed, hasSupportPage, liveWord, noTokenHelp, noWorkspaceText, shellTitle, showFirstRun, supportHref } from "../src/core/firstRun.ts";
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

test("the sign-in card never sends a stranger to People for a first token", () => {
  // A browser session cannot mint in People; the first token comes from Tools, then Connect.
  assert.match(noTokenHelp(true), /Tools, then Connect/);
  assert.doesNotMatch(noTokenHelp(true), /under People/);
  assert.doesNotMatch(noTokenHelp(false), /under People/);
  assert.match(noWorkspaceText(false), /isn't a member of any workspace/);
  assert.doesNotMatch(noWorkspaceText(false), /create/i);
  assert.match(noWorkspaceText(true), /with an invite/);
});

test("the first-run guide shows on a loaded empty board until it is put away, per workspace", () => {
  assert.equal(showFirstRun("ready", 0, false), true);
  assert.equal(showFirstRun("loading", 0, false), false);
  assert.equal(showFirstRun("ready", 1, false), false);
  assert.equal(showFirstRun("ready", 0, true), false);
  const kept = new Map<string, string>();
  const store = { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
  assert.equal(firstRunDismissed("w1", store), false);
  dismissFirstRun("w1", store);
  dismissFirstRun("w1", store);
  assert.equal(firstRunDismissed("w1", store), true);
  assert.equal(firstRunDismissed("w2", store), false);
  const blocked = { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); } };
  assert.equal(firstRunDismissed("w1", blocked), false);
  assert.doesNotThrow(() => dismissFirstRun("w1", blocked));
});

test("the Support link appears only where the origin serves a support page", async () => {
  const page = async () => new Response(null, { status: 200, headers: { "content-type": "text/html; charset=utf-8" } });
  const missing = async () => new Response(null, { status: 404, headers: { "content-type": "application/problem+json" } });
  const down = async (): Promise<Response> => { throw new TypeError("offline"); };
  let asked = "";
  assert.equal(await hasSupportPage("https://maidan.example/", async (url, init) => { asked = `${init?.method} ${url}`; return page(); }), true);
  assert.equal(asked, "HEAD https://maidan.example/support");
  assert.equal(await hasSupportPage("https://self.example", missing), false);
  assert.equal(await hasSupportPage("https://self.example", down), false);
  assert.equal(supportHref("https://maidan.example/"), "https://maidan.example/support");
});
