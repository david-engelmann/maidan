import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

// Open a group DM with two other members and post into it. The signed-in
// person is added, which makes the three members the store requires.
test("opening a group DM selects it and a message shows in the conversation", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="group-dms"]');

  await page.fill("#gdm-member-ids", `${fx.requester_id}, ${fx.lab_member_id}`);
  await page.fill("#gdm-title", "release desk");
  await page.click("#gdm-open");
  await expect(page.locator("#gdm-selected")).toContainText("release desk");
  await expect(page.locator("#toasts .toast-success", { hasText: "Group DM opened" })).toHaveAttribute("role", "status");

  const body = `hello group ${Date.now()}`;
  await page.fill("#gdm-body", body);
  await page.click("#gdm-send");
  await expect(page.locator("#gdm-messages")).toContainText(body);
  await expect(page.locator("#gdm-messages")).toContainText("Operator");
});
