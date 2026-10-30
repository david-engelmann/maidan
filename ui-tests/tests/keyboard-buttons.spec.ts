import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { callUiExport } from "./_tools";

const fx = fixtures();

// Board cards and Needs-you rows used to be DIVs and ARTICLEs with
// role="button" and hand-rolled Enter/Space handlers. They are real buttons
// now, and a board refresh that rebuilds the focused card hands focus to its
// new node instead of dropping it to <body>.

async function openBoard(page: Page) {
  const session = page.waitForResponse(
    (r) => r.request().method() === "GET" && r.url().endsWith("/auth/session"),
  );
  await page.goto("/ui/");
  await session;
  await page.fill("#workspace", fx.workspace_id);
  const channels = page.waitForResponse(
    (r) => r.request().method() === "GET" && r.url().includes(`/workspaces/${fx.workspace_id}/channels`) && r.ok(),
  );
  await page.fill("#token", fx.review_token);
  await page.locator("#token").blur();
  await channels;
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await expect(page.locator("#board-title")).toHaveText("# build");
  await expect(page.locator(`#board .card[data-id="${fx.board_review_thread_id}"]`)).toBeVisible();
  await expect(page.locator("#needs-you-list .ny-item").first()).toBeVisible();
}

test("cards and Needs-you rows open through real buttons, not role=button DIVs", async ({ page }) => {
  await openBoard(page);
  // Nothing on the board or in Needs you fakes a button any more.
  await expect(page.locator('#board [role="button"], #needs-you-list [role="button"], #needs-you-list [role="link"]')).toHaveCount(0);
  await expect(page.locator("#board .card[tabindex]")).toHaveCount(0);
  const cards = await page.locator("#board .card").count();
  await expect(page.locator("#board .card > button.card-open")).toHaveCount(cards);
  const opening = page.locator("#needs-you-list .ny-item[data-thread-id] .ny-title");
  expect(await opening.count()).toBeGreaterThan(0);
  for (const tag of await opening.evaluateAll((els) => els.map((e) => e.tagName))) expect(tag).toBe("BUTTON");

  // The card's button carries the card's name and state, and Space presses it.
  const open = page.locator(`#board .card[data-id="${fx.board_review_thread_id}"] .card-open`);
  await expect(open).toHaveAccessibleName(/: in review/);
  await open.focus();
  await page.keyboard.press(" ");
  await expect(page.locator("#thread-context")).toHaveText("Review: result waiting on a reviewer");

  // A Needs-you row's title is a button that Enter presses.
  const row = page.locator(`#needs-you-list .ny-item[data-thread-id="${fx.board_review_thread_id}"] .ny-title`).first();
  if (await row.count()) {
    await page.locator(`#board .card[data-id="${fx.board_open_thread_id}"] .card-open`).click();
    await row.focus();
    await page.keyboard.press("Enter");
    await expect(page.locator("#thread-context")).toHaveText("Review: result waiting on a reviewer");
  }
});

test("a keyboard walk never lands on BODY and every stop is a real control", async ({ page }) => {
  await openBoard(page);
  await page.locator("body").evaluate((b) => (b as HTMLElement).focus());
  const stops: string[] = [];
  for (let i = 0; i < 160; i++) {
    await page.keyboard.press("Tab");
    const stop = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      if (!el) return "NONE";
      const role = el.getAttribute("role") || "";
      return `${el.tagName}${role ? `[${role}]` : ""}`;
    });
    stops.push(stop);
  }
  // Tab past the last control wraps through the browser chrome, which a page
  // sees as BODY for one press; never two in a row, and never mid-page.
  const firstBody = stops.indexOf("BODY");
  const walk = firstBody === -1 ? stops : stops.slice(0, firstBody);
  expect(walk.length, stops.join(" ")).toBeGreaterThan(20);
  expect(walk.filter((s) => s === "BODY" || s === "NONE")).toEqual([]);
  const fake = walk.filter((s) => /^(DIV|ARTICLE|SPAN)\[(button|link)\]$/.test(s));
  expect(fake, walk.join(" ")).toEqual([]);
});

test("a board refresh that rebuilds the focused card keeps focus on that card", async ({ page }) => {
  await openBoard(page);
  const id = fx.board_review_thread_id;
  const open = page.locator(`#board .card[data-id="${id}"] .card-open`);
  await open.focus();
  await expect(open).toBeFocused();
  const before = await open.elementHandle();

  // The next threads read retitles the task, so its card is a new node.
  await page.route(
    (url) => url.pathname.endsWith(`/channels/${fx.board_channel_id}/threads`),
    async (route) => {
      const res = await route.fetch();
      const body = await res.json();
      for (const th of body) if (th.id === id) th.title = `${th.title || ""} (renamed)`;
      await route.fulfill({ response: res, json: body });
    },
  );
  await callUiExport(page, "board.js", "loadThreads");

  await expect(open).toContainText("(renamed)");
  await expect(open).toBeFocused();
  expect(await page.evaluate((el) => el!.isConnected, before)).toBe(false);
  expect(await page.evaluate(() => document.activeElement?.tagName)).toBe("BUTTON");
});
