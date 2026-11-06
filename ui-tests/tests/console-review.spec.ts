import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";
import { laidOutBox } from "./_layout";

// Needs you on a wide screen: a queue on the left, the picked item's evidence
// and actions on the right, and a and r acting on the picked item.
const fx = fixtures();

test("a wide screen reviews from a queue, and r sends back the picked item", async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 1000 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.tiers_channel_id, "#/needs");
  const picks = page.locator(".need-pick");
  await expect(picks.nth(1)).toBeVisible();
  await expect(page.locator(".need-card:visible")).toHaveCount(1);
  const title = (await picks.nth(1).locator(".need-pick-title").textContent()) ?? "";
  await picks.nth(1).click();
  await expect(picks.nth(1)).toHaveAttribute("aria-current", "true");
  const pane = page.locator(".need.selected .need-card");
  await expect(pane.locator(".need-title")).toHaveText(title);
  const pickBox = await laidOutBox(picks.nth(1));
  const paneBox = await laidOutBox(pane);
  expect(paneBox.x, "the detail sits to the right of the queue").toBeGreaterThan(pickBox.x + pickBox.width);
  await page.locator("body").press("r");
  await expect(pane.locator("textarea")).toBeFocused();
  await pane.getByRole("button", { name: "Cancel" }).click();
});

test("a narrow screen keeps one column of full cards", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.tiers_channel_id, "#/needs");
  await expect(page.locator(".need-card").first()).toBeVisible();
  await expect(page.locator(".need-pick:visible")).toHaveCount(0);
  expect(await page.locator(".need-card:visible").count()).toBeGreaterThan(1);
});

test("the wide pane is full height with the result inline and a footer that stays in view", async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 1000 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.tiers_channel_id, "#/needs");
  const pane = page.locator(".need.selected .need-card");
  await expect(pane.locator(".rc-result .rc-head")).toHaveText("Result");
  await expect(pane.locator(".rc-recent .rc-head")).toHaveText("Recent messages");
  await expect(pane.locator(".rc-result")).not.toContainText("Loading");
  await expect(pane.getByRole("link", { name: "Open full thread" })).toBeVisible();
  const box = await laidOutBox(pane);
  expect(box.height, "the pane fills the screen's height").toBeGreaterThan(800);
  const foot = await laidOutBox(pane.locator(":scope > .need-actions"));
  expect(foot.y + foot.height).toBeLessThanOrEqual(1000);
  // A queue title clamps to two lines and the waiting time sits on the right.
  const pick = page.locator(".need-pick").first();
  expect(await pick.locator(".need-pick-title").evaluate((e) => getComputedStyle(e).webkitLineClamp)).toBe("2");
  const age = await laidOutBox(pick.locator(".need-pick-age"));
  const pb = await laidOutBox(pick);
  expect(pb.x + pb.width - (age.x + age.width)).toBeLessThan(20);
  expect(await pick.locator(".need-pick-age").evaluate((e) => getComputedStyle(e).fontVariantNumeric)).toContain("tabular-nums");
});

test("j and k walk the queue, and an answer moves on to the next item", async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 1000 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.tiers_channel_id, "#/needs");
  const picks = page.locator(".need-pick");
  await expect(picks.nth(1)).toBeVisible();
  await expect(picks.nth(0)).toHaveAttribute("aria-current", "true");
  await page.locator("body").press("j");
  await expect(picks.nth(1)).toHaveAttribute("aria-current", "true");
  await page.keyboard.press("k");
  await expect(picks.nth(0)).toHaveAttribute("aria-current", "true");
  const first = (await picks.nth(0).locator(".need-pick-title").textContent()) ?? "";
  const second = (await picks.nth(1).locator(".need-pick-title").textContent()) ?? "";
  // The answer is faked so the shared seed stays as other specs expect it.
  await page.route(/\/threads\/[^/]+\/reviews$/, (route) => (route.request().method() === "POST" ? route.fulfill({ status: 201, json: {} }) : route.continue()));
  await page.keyboard.press("r");
  const pane = page.locator(".need.selected .need-card");
  await pane.locator("textarea").fill("Please add the log");
  await pane.locator("button[type=submit]").click();
  await expect(page.locator(".need.selected .need-card .need-title")).toHaveText(second);
  await expect(page.locator(".need-pick[aria-current='true'] .need-pick-title")).not.toHaveText(first);
});

test("a screenshot in the evidence shows as a thumbnail in the pane", async ({ page }) => {
  // The seeded screenshot's bytes are filler, so serve a small real PNG in its place.
  const png = readFileSync(join(__dirname, "assets", "login-after.png"));
  await page.route(/\/artifacts\/e4fd[0-9a-f]{60}$/, (r) => r.fulfill({ status: 200, contentType: "image/png", body: png }));
  await page.setViewportSize({ width: 1680, height: 1000 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.tiers_channel_id, "#/needs");
  await page.locator(".need-pick", { hasText: "screenshot and a log" }).click();
  const thumb = page.locator(".need.selected .rc-files img.rc-thumb");
  await expect(thumb).toBeVisible();
  await expect(thumb).toHaveAttribute("src", /^blob:/);
  await expect.poll(() => thumb.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBe(160);
});
