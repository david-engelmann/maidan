import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

// Notification prefs are self-only. The admin token is the operator, so the
// session it exchanges for is the member the panel reads and writes.
test("prefs sets delivery mode, email, a mute, and a channel follow", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="prefs"]');

  const delivery = page.locator("#prefs-delivery");
  await expect(delivery).toContainText("Delivery mode:");
  await page.click("#prefs-mode-digest");
  await expect(delivery).toContainText("Delivery mode: digest");

  const email = "operator@example.test";
  await page.fill("#prefs-email", email);
  await page.click("#prefs-email-set");
  await expect(page.locator("#prefs-email-current")).toContainText(email);

  await page.click("#prefs-mute");
  await expect(page.locator("#prefs-mute-list")).toContainText("message_posted");

  await page.fill("#prefs-follow-channel", fx.channel_id);
  await page.click("#prefs-follow-channel-btn");
  await expect(page.locator("#prefs-channel-follows")).toContainText(fx.channel_id);
});
