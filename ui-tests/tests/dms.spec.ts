import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

// Open a DM with the seeded Deployer and post into it. The conversation
// names the other person, not a raw id.
test("opening a DM selects it and a message shows in the conversation", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="dms"]');

  await page.fill("#dm-other-id", fx.requester_id);
  await page.click("#dm-open");
  await expect(page.locator("#dm-selected")).toContainText("Deployer");
  await expect(page.locator("#status")).toHaveText("DM opened");

  const body = `hello from the board ${Date.now()}`;
  await page.fill("#dm-body", body);
  await page.click("#dm-send");
  await expect(page.locator("#dm-messages")).toContainText(body);
  await expect(page.locator("#dm-messages")).toContainText("Operator");
});
