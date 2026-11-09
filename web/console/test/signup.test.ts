import { test } from "node:test";
import assert from "node:assert/strict";
import { isWorkspaceInvite, readSignupCheck, sameOriginReturn, turnstileSize, workspaceNameRefusal, WORKSPACE_NAME_MAX } from "../src/core/signup.ts";

test("a blank workspace name is allowed and a bad one is not", () => {
  assert.equal(workspaceNameRefusal(""), null);
  assert.equal(workspaceNameRefusal("   "), null);
  assert.equal(workspaceNameRefusal("  North Room  "), null);
  assert.equal(workspaceNameRefusal("n".repeat(WORKSPACE_NAME_MAX)), null);
  assert.match(workspaceNameRefusal("n".repeat(WORKSPACE_NAME_MAX + 1)) ?? "", /200/);
  assert.match(workspaceNameRefusal("North\nRoom") ?? "", /control/);
  assert.match(workspaceNameRefusal("North\u0000Room") ?? "", /control/);
  assert.match(workspaceNameRefusal("North\u009fRoom") ?? "", /control/);
});

test("the sign-up check keeps the name, the invite and a same-origin return", () => {
  const check = readSignupCheck("?signup_check=1&workspace_name=North%20Room&invite=tok&return_to=%2Fui%2F");
  assert.equal(check.active, true);
  assert.equal(check.name, "North Room");
  assert.equal(check.invite, "tok");
  assert.equal(check.returnTo, "/ui/");
  assert.equal(isWorkspaceInvite(check.invite), false);
  assert.equal(isWorkspaceInvite("maid_inv_abc"), true);
  assert.deepEqual(readSignupCheck(""), { active: false, name: "", invite: null, returnTo: "/ui/" });
  assert.equal(sameOriginReturn("//evil.example"), "/ui/");
  assert.equal(sameOriginReturn("https://evil.example"), "/ui/");
  assert.equal(sameOriginReturn("/ui/?x=1"), "/ui/?x=1");
});

test("the sign-up check is compact in a column narrower than its normal widget", () => {
  // The sign-in card's column is 275px on a 360px phone and 297px at 390.
  assert.equal(turnstileSize(275), "compact");
  assert.equal(turnstileSize(299), "compact");
  assert.equal(turnstileSize(300), "normal");
  assert.equal(turnstileSize(342), "normal");
});
