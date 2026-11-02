import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// The board in depth: a task drawer with a deep link, filters kept in the
// URL, and c for a new task.
const fx = fixtures();

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 1000 });
  await signInWithCookie(page, fx);
});

test("a card opens a drawer that is a deep link, and Escape closes it", async ({ page }) => {
  await openOnChannel(page, fx.tiers_channel_id);
  const card = page.locator(".lane .task").first();
  await expect(card).toBeVisible();
  const title = (await card.locator(".task-title").textContent()) ?? "";
  await card.click();
  const sheet = page.getByRole("dialog");
  await expect(sheet.locator("#sheet-title")).toHaveText(title);
  await expect(sheet.getByText("State", { exact: true })).toBeVisible();
  await expect(sheet.locator(".rc-head", { hasText: "Recent messages" })).toBeVisible();
  await expect(sheet.getByRole("link", { name: "Open full thread" })).toBeVisible();
  await expect(page).toHaveURL(/#\/board\?task=[0-9a-f-]{36}$/);
  const link = page.url();

  await page.keyboard.press("Escape");
  await expect(sheet).toHaveCount(0);
  await expect(page).toHaveURL(/#\/board$/);

  await page.goto(link);
  await expect(page.getByRole("dialog").locator("#sheet-title")).toHaveText(title);
});

test("filters narrow the board and live in the URL", async ({ page }) => {
  await openOnChannel(page, fx.tiers_channel_id);
  const cards = page.locator(".lane .task");
  await expect(page.locator(".board-summary")).toContainText(/\d+ tasks?/);
  const all = Number(/(\d+) task/.exec((await page.locator(".board-summary").textContent()) ?? "")?.[1]);
  await expect(cards).toHaveCount(all);
  const title = ((await cards.first().locator(".task-title").textContent()) ?? "").trim();

  await page.getByRole("searchbox", { name: "Search tasks" }).fill(title);
  await expect(page).toHaveURL(/[?&]q=/);
  await expect(cards.filter({ hasText: title }).first()).toBeVisible();
  await expect.poll(() => cards.count()).toBeLessThan(all);
  await expect(page.locator(".filter-count")).toContainText(`of ${all}`);

  await page.getByLabel("State").selectOption("done");
  await expect(page).toHaveURL(/state=done/);
  await expect(page.locator(".lane")).toHaveCount(1);

  await page.reload();
  await expect(page.getByLabel("State")).toHaveValue("done");
  await expect(page.getByRole("searchbox", { name: "Search tasks" })).toHaveValue(title);

  await page.getByRole("button", { name: "Clear" }).click();
  await expect(page).toHaveURL(/#\/board$/);
  await expect(cards).toHaveCount(all);
});

test("c opens the composer and a new task opens in the drawer", async ({ page }) => {
  await openOnChannel(page, fx.tiers_channel_id);
  const first = page.locator(".lane .task").first();
  await expect(first).toBeVisible();
  // Answer the create with an existing task, so the shared seed stays as it is.
  await first.click();
  const id = new URL(page.url()).hash.split("task=")[1];
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  let body: unknown = null;
  await page.route(/\/channels\/[^/]+\/threads$/, (r) => {
    if (r.request().method() !== "POST") return r.fallback();
    body = r.request().postDataJSON();
    return r.fulfill({ status: 201, json: { id } });
  });

  await page.keyboard.press("c");
  const field = page.getByPlaceholder("What needs doing?");
  await expect(field).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(field).toHaveCount(0);

  await page.getByRole("button", { name: /New task/ }).click();
  await field.fill("Write the release notes");
  await page.getByRole("button", { name: "Create" }).click();
  expect(body).toEqual({ title: "Write the release notes" });
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`task=${id}`));
});
