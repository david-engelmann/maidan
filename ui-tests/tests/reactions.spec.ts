import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

// A bare emoji is read out by its Unicode name with nothing about what the
// button does, so each reaction control says the reaction and the action, and
// a chip says whether it is yours.
test("reaction controls name the reaction and what pressing them does", async ({ page }) => {
  const body = `React to me ${Date.now().toString(36)}`;
  const posted = await page.request.post(`${fx.base_url}/threads/${fx.thread_id}/messages`, {
    headers: { Authorization: `Bearer ${fx.token}` },
    data: { body },
  });
  expect(posted.ok()).toBeTruthy();

  await signIn(page, fx.workspace_id, fx.token);
  await page.click("#refresh-channels");
  await page.click(`#channel-list li[data-id="${fx.channel_id}"]`);
  await page.click(`#board .card[data-id="${fx.thread_id}"]`);

  const msg = page.locator("#message-list .msg", { hasText: body });
  const add = msg.locator(".reaction-add", { hasText: "👍" });
  await expect(add).toHaveAttribute("aria-label", "React with thumbs up");
  await expect(msg.locator(".reaction-add", { hasText: "👀" })).toHaveAttribute("aria-label", "React with eyes");

  // The quick reactions show on hover or focus within the message.
  await msg.hover();
  await add.click();
  const chip = msg.locator(".reaction-chip", { hasText: "👍" });
  await expect(chip).toHaveAttribute("aria-label", "Remove your thumbs up reaction, 1 so far");
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expect(msg.getByRole("button", { name: "Remove your thumbs up reaction, 1 so far" })).toBeVisible();

  await chip.click();
  await expect(msg.locator(".reaction-chip", { hasText: "👍" })).toHaveCount(0);
});
