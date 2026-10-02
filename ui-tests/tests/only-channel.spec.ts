import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

async function signIn(page: Page) {
  await page.goto("/ui/");
  await page.evaluate(() => localStorage.removeItem("maidan_channel"));
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.locator("#token").dispatchEvent("change");
}

// Several channels: nothing is chosen for the viewer. The empty board is
// one sentence and one action, not a prompt to pick a channel.
test("several channels stay unchosen until one is chosen", async ({ page }) => {
  await signIn(page);
  await expect(page.locator("#channel-list li[data-id]").first()).toBeVisible();
  const onboard = page.locator("#board-onboard");
  await expect(onboard).toContainText("A task arrives when an agent or a person opens one.");
  await expect(onboard.getByRole("button", { name: "Connect an agent" })).toBeVisible();
  await expect(onboard).not.toContainText("pick a channel");
  await expect(page.locator("#channel-list li.selected")).toHaveCount(0);
  await expect(page.locator("#board .board-col")).toHaveCount(0);
});

// One channel is the board. The viewer should not have to click it while a
// Needs you queue is already live above an empty stage.
test("the only channel opens by itself", async ({ page }) => {
  await page.route(/\/workspaces\/[^/]+\/channels$/, async (route) => {
    // A signed-in load goes through the session proxy (/ui/api) and is an
    // array. A probe that is not that list is left alone. Keep only the
    // empty channel so the board opens it by itself.
    if (route.request().method() !== "GET") return route.continue();
    const res = await route.fetch({ headers: route.request().headers() });
    const text = await res.text();
    let all: unknown;
    try {
      all = JSON.parse(text);
    } catch {
      all = null;
    }
    if (!Array.isArray(all)) {
      await route.fulfill({
        status: res.status(),
        contentType: "application/json",
        body: text,
      });
      return;
    }
    const one = all.filter((c: { id: string }) => c.id === fx.quiet_channel_id);
    await route.fulfill({
      status: res.status(),
      contentType: "application/json",
      body: JSON.stringify(one),
    });
  });
  await signIn(page);
  const row = page.locator(`#channel-list li[data-id="${fx.quiet_channel_id}"]`);
  await expect(row).toHaveClass(/selected/);
  await expect(page.locator("#board-title")).toHaveText("# quiet");
  const onboard = page.locator("#board-onboard");
  await expect(onboard).toContainText("A task arrives when an agent or a person opens one.");
  await expect(onboard.getByRole("button", { name: "Connect an agent" })).toBeVisible();
  await expect(onboard).not.toContainText("pick a channel");
  await expect(onboard).not.toContainText("No tasks in #quiet");
  await expect(page.locator("#board .board-col")).toHaveCount(0);
});
