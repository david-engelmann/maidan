import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// History, blocking dependencies, and a member's work across channels.
const fx = fixtures();

async function noOverflow(page: Page): Promise<void> {
  const sideways = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(sideways).toBeLessThanOrEqual(1);
}

async function axeClean(page: Page, include: string): Promise<void> {
  const root = page.locator(include);
  await expect.poll(() => root.evaluate((el) => getComputedStyle(el).opacity)).toBe("1");
  const r = await new AxeBuilder({ page }).include(include).withTags(["wcag2a", "wcag2aa"]).analyze();
  const bad = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(bad.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join("; ")}`)).toEqual([]);
}

async function dependReviewOnDone(page: Page): Promise<void> {
  const res = await page.request.post(`/threads/${fx.board_review_thread_id}/dependencies`, {
    headers: { Authorization: `Bearer ${fx.review_token}`, "content-type": "application/json" },
    data: { depends_on_thread_id: fx.board_done_thread_id },
  });
  expect(res.status(), await res.text()).toBe(204);
}

async function openReview(page: Page): Promise<void> {
  const card = page.locator("button.task", { hasText: "Review: result waiting" });
  await expect(card).toBeVisible();
  await card.click();
}

test("the task sheet lists history and the tasks it depends on", async ({ page }) => {
  await dependReviewOnDone(page);
  await page.setViewportSize({ width: 1680, height: 1000 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id);
  await openReview(page);
  const sheet = page.getByRole("dialog");
  await expect(sheet).toBeVisible();
  const summary = sheet.locator("summary");
  await expect(summary).toContainText(/History \(\d+\)/);
  const dep = sheet.getByRole("link", { name: "Done: closed" });
  await expect(dep).toBeVisible();
  await expect(dep).toHaveAttribute("href", `#/threads/t/${fx.board_done_thread_id}`);
  await expect(sheet.getByText("(done)")).toBeVisible();

  await expect(sheet.getByRole("button", { name: "Close the task sheet" })).toBeFocused();
  for (let i = 0; i < 25 && !(await summary.evaluate((el) => el === document.activeElement)); i++) {
    await page.keyboard.press("Tab");
    const inside = await page.evaluate(() => {
      const dialog = document.querySelector("[role=dialog]");
      return !!dialog && dialog.contains(document.activeElement);
    });
    expect(inside).toBe(true);
  }
  await expect(summary).toBeFocused();
  await page.keyboard.press("Enter");
  const history = sheet.locator("summary + ul");
  await expect(history).toContainText("Deployer");
  await expect(history).toContainText("Send to review");
  await expect(history.locator("time").first()).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();

  await openReview(page);
  await expect(sheet).toBeVisible();
  await noOverflow(page);
  await axeClean(page, ".sheet");
});

test.describe("task sheet on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("history and dependencies fit, and Escape closes the sheet", async ({ page }) => {
    await dependReviewOnDone(page);
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id);
    await page.getByRole("tab", { name: /Needs review/ }).click();
    await openReview(page);
    const sheet = page.getByRole("dialog");
    await expect(sheet.locator("summary")).toContainText(/History \(\d+\)/);
    await expect(sheet.getByRole("link", { name: "Done: closed" })).toBeVisible();
    await noOverflow(page);
    await axeClean(page, ".sheet");
    await page.keyboard.press("Tab");
    const inside = await page.evaluate(() => {
      const dialog = document.querySelector("[role=dialog]");
      return !!dialog && dialog.contains(document.activeElement);
    });
    expect(inside).toBe(true);
    await page.keyboard.press("Escape");
    await expect(sheet).toBeHidden();
  });
});

test("a member's page shows work from every channel, when they were last active, and opens a message", async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 1000 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, `#/people/m/${fx.requester_id}`);
  const work = page.getByRole("region", { name: "Work" });
  await expect(work.getByRole("heading", { name: "Work", exact: true })).toBeVisible();
  await expect(work.getByRole("heading", { name: "Work in this channel" })).toHaveCount(0);
  await expect(work.getByRole("heading", { name: "Now" })).toBeVisible();
  await expect(work.locator(".work-list a").first()).toHaveAttribute("href", /#\/threads\/t\//);
  await expect(work.locator(".work-list .muted").first()).toContainText(/#[a-z0-9-]+/i);
  const facts = page.locator(".member-detail .facts");
  await expect(facts.locator("dt").first()).toHaveText("Last active");
  await expect(facts.locator("dt").last()).toHaveText("Member id");
  const last = facts.locator("div").filter({ has: page.locator("dt", { hasText: /^Last active$/ }) });
  await expect(last.locator("dd")).toHaveText(/Online now|Away|ago|just now|No activity yet/);
  await page.getByRole("button", { name: "Message" }).click();
  await expect(page).toHaveURL(/#\/threads\/dm\/[0-9a-f-]{36}/);
});

test.describe("people on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("Message is a 44px target and the member page has no sideways scroll", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id, `#/people/m/${fx.requester_id}`);
    const button = page.getByRole("button", { name: "Message" });
    await expect(button).toBeVisible();
    const box = await button.boundingBox();
    expect(box && box.height).toBeGreaterThanOrEqual(44);
    expect(box && box.width).toBeGreaterThanOrEqual(44);
    await noOverflow(page);
    await axeClean(page, ".member-detail");
  });
});
