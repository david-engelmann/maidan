import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { mkdirSync } from "fs";
import { resolve } from "path";

// Capture the README/docs screenshots from the same seeded harness the rest of
// the suite uses, so a picture in the docs is a picture of a working build with
// deterministic fixtures — not a staged screenshot that drifts from the UI and
// that nobody notices has drifted.
//
// Skipped unless asked for: `UI_SHOOT=1 npx playwright test screenshots`.
// It is a capture step, not an assertion, and a docs refresh should not be able
// to fail the required `ui tests (playwright)` job.
const SHOOT = process.env.UI_SHOOT === "1";
const OUT = resolve(__dirname, "../../docs/assets");

test.describe("docs screenshots", () => {
  test.skip(!SHOOT, "set UI_SHOOT=1 to re-capture the docs screenshots");

  const fx = fixtures();

  test.beforeAll(() => mkdirSync(OUT, { recursive: true }));

  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/ui/");
    await page.fill("#workspace", fx.workspace_id);
    await page.fill("#token", fx.token);
    await page.click("#refresh-channels");
    await expect(
      page.locator("#channel-list li").filter({ hasText: "general" }),
    ).toBeVisible();
  });

  test("the workspace a team of agents shares", async ({ page }) => {
    await page.click('.tabs button[data-tab="admin"]');
    await page.click("#channel-list li");
    await expect(page.locator("#thread-list li").first()).toBeVisible();
    await page.screenshot({ path: `${OUT}/ui-workspace.png` });
  });

  test("the work console: who holds what, and for how long", async ({ page }) => {
    await page.click('.tabs button[data-tab="work"]');
    await expect(page.locator("#panel-work")).toBeVisible();
    await page.screenshot({ path: `${OUT}/ui-work.png` });
  });

  test("the looking glass: the event log as it happened", async ({ page }) => {
    await page.click('.tabs button[data-tab="glass"]');
    await expect(page.locator("#panel-glass")).toBeVisible();
    await page.screenshot({ path: `${OUT}/ui-glass.png` });
  });
});
