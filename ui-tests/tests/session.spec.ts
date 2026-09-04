import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// The capability card in a real browser (Cluster 353.1): the Session tab reads
// GET /me and renders what the token actually carries. The seeded fixture token
// holds workspace:read + workspace:write + message:post, so those land in "Can"
// and the rest of the vocabulary (e.g. token:admin) lands in "Can't".
test("the Session tab renders the capability card from the real grant", async ({ page }) => {
  await page.goto("/ui/");
  // A bearer is enough for /me; the tab loads on click.
  await page.fill("#token", fx.token);
  await page.click('.tabs button[data-tab="session"]');

  // Identity: a bearer acts as any member (the orchestrator model).
  await expect(page.locator("#session-credential")).toContainText("bearer");
  await expect(page.locator("#session-member")).toHaveText(fx.member_id);

  // "Can" holds exactly the three granted capabilities.
  const can = page.locator("#cap-can-list");
  await expect(page.locator("#cap-can-count")).toHaveText("(3)");
  await expect(can.locator("li", { hasText: "workspace:read" })).toBeVisible();
  await expect(can.locator("li", { hasText: "workspace:write" })).toBeVisible();
  await expect(can.locator("li", { hasText: "message:post" })).toBeVisible();

  // "Can't" holds a withheld capability — the card's whole point.
  const cant = page.locator("#cap-cant-list");
  await expect(cant.locator("li", { hasText: "token:admin" })).toBeVisible();
  await expect(cant.locator("li", { hasText: "channel:admin" })).toBeVisible();
});
