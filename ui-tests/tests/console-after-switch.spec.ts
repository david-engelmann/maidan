import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// The checks the classic board held, carried onto the console after the
// switch: a skip link, reduced motion, 44px targets, a quiet empty Needs you
// with one primary action per row, keyboard focus that lands somewhere real,
// the palette by keyboard, the member picker, deliveries replay, a missing
// artifact, a workspace with one channel, and the phone header's labels.
const fx = fixtures();

test("the skip link is first in tab order and moves focus to the content", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id);
  await expect(page.locator(".lane").first()).toBeVisible();
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to content" });
  await expect(skip).toBeFocused();
  await expect(skip).toBeInViewport();
  await page.keyboard.press("Enter");
  await expect(page.locator("main#main")).toBeFocused();
  await expect(page).toHaveURL(/#\/board/);
});

test.describe("with reduced motion", () => {
  test.use({ reducedMotion: "reduce" });

  test("nothing transitions or animates", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id);
    const task = page.locator("button.task").first();
    await expect(task).toBeVisible();
    const style = await task.evaluate((el) => {
      const s = getComputedStyle(el);
      return { transition: s.transitionDuration, animation: s.animationDuration };
    });
    expect(style.transition.split(",").every((d) => parseFloat(d) === 0)).toBe(true);
    expect(style.animation.split(",").every((d) => parseFloat(d) === 0)).toBe(true);
  });
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  for (const hash of ["#/board", "#/needs", "#/threads", "#/people", "#/tools/deliveries"]) {
    test(`every visible button on ${hash} is at least 44px tall`, async ({ page }) => {
      await signInWithCookie(page, fx);
      await openOnChannel(page, fx.board_channel_id, hash);
      await expect(page.locator(".tabbar")).toBeVisible();
      await page.waitForLoadState("networkidle");
      const small = await page.locator("button:visible, a.btn:visible, .seg a:visible").evaluateAll((els) =>
        els
          .map((el) => ({ el, r: el.getBoundingClientRect() }))
          .filter(({ r }) => r.width > 0 && r.height > 0 && r.height < 43.5)
          .map(({ el, r }) => `${el.tagName.toLowerCase()}.${el.className} "${(el.textContent ?? "").trim().slice(0, 30)}" ${Math.round(r.height)}px`),
      );
      expect(small, small.join("\n")).toEqual([]);
    });
  }

  test("the header names the page and the tab bar's tabs carry labels", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id, "#/needs");
    await expect(page.locator("header.top h1")).toHaveText(/Needs you/);
    await expect(page.getByRole("button", { name: /Search and commands/ })).toBeVisible();
    const labels = await page.locator(".tabbar .label").allTextContents();
    expect(labels.length).toBeGreaterThanOrEqual(3);
    for (const l of labels) expect(l.trim()).not.toBe("");
    await page.locator(".tabbar a", { hasText: "Board" }).click();
    await expect(page.locator("header.top h1")).toHaveText(/Board/);
  });
});

test("an empty Needs you is calm: one quiet card, no buttons", async ({ page }) => {
  await page.route(/\/members\/[^/]+\/waiting/, (route) => route.fulfill({ json: [] }));
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/needs");
  const empty = page.locator(".needs-empty");
  await expect(empty).toContainText("All clear");
  await expect(page.locator(".needs-count")).toHaveText(/Nothing waiting/);
  await expect(empty.locator("button")).toHaveCount(0);
});

test("each Needs you row offers at most one primary action", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/needs");
  await expect(page.locator(".needs-count")).toBeVisible();
  const rows = page.locator(".need-actions");
  test.skip((await rows.count()) === 0, "nothing waiting in this seed");
  for (const n of await rows.evaluateAll((els) => els.map((e) => e.querySelectorAll(".btn.go").length))) {
    expect(n).toBeLessThanOrEqual(1);
  }
});

test("Tab reaches a Needs you row's actions", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/needs");
  await expect(page.locator(".needs-count")).toBeVisible();
  const first = page.locator(".need-actions button").first();
  test.skip((await first.count()) === 0, "nothing waiting in this seed");
  await first.focus();
  await expect(first).toBeFocused();
  await expect(first).toHaveCSS("outline-style", "solid");
});

test("the palette opens on the keyboard and goes where Enter says", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id);
  await expect(page.locator(".lane").first()).toBeVisible();
  await page.keyboard.press("ControlOrMeta+k");
  const box = page.getByRole("dialog");
  await expect(box).toBeVisible();
  await page.keyboard.type("people");
  await expect(box.getByRole("option").first()).toContainText(/People/);
  await page.keyboard.press("Enter");
  await expect(box).toBeHidden();
  await expect(page).toHaveURL(/#\/people/);
});

test("the member picker finds people by handle and says who is an agent", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/threads");
  await page.getByRole("tab", { name: "Direct" }).click();
  await page.getByRole("button", { name: "New message" }).click();
  const list = page.getByRole("list", { name: "People" });
  await expect(list.locator("li").first()).toBeVisible();
  await expect(list.locator(".tag", { hasText: /^(Agent|Person)$/i }).first()).toBeVisible();
  await page.getByLabel(/Find a person or agent/).fill("zz-nobody-has-this-handle");
  await expect(list.getByRole("checkbox")).toHaveCount(0);
  await expect(list.getByText("No one matches.")).toBeVisible();
  await page.getByLabel(/Find a person or agent/).fill("");
  const boxes = list.getByRole("checkbox");
  test.skip((await boxes.count()) < 2, "needs two other members");
  await boxes.nth(0).check();
  await expect(page.getByRole("button", { name: "Start conversation" })).toBeVisible();
  await boxes.nth(1).check();
  await expect(page.getByLabel(/Group name/)).toBeVisible();
  await expect(page.getByRole("button", { name: /Start group of/ })).toBeVisible();
});

test("a dead delivery can be replayed from Tools", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/tools/deliveries");
  const row = page.locator("li", { hasText: `#${fx.delivery_id}` }).first();
  const shown = await row.waitFor({ timeout: 8_000 }).then(() => true, () => false);
  test.skip(!shown, "no dead letter in this seed for this member");
  await expect(row).toContainText("Dead letter");
  await row.getByRole("button", { name: "Replay" }).click();
  await expect(page.getByText(new RegExp(`Replayed .* #${fx.delivery_id}`))).toBeVisible();
});

test("an artifact that is not there says so, without a crash", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/artifacts");
  const field = page.getByLabel("sha256");
  await field.fill("0".repeat(64));
  await field.press("Enter");
  await expect(page.getByText("Not found, or not in this workspace.")).toBeVisible();
});

test("a workspace with one channel still shows the board, calmly", async ({ page }) => {
  await page.route(/\/workspaces\/[^/]+\/channels(\?.*)?$/, async (route) => {
    const res = await route.fetch();
    const rows = (await res.json()) as Array<{ id: string }>;
    await route.fulfill({ response: res, json: rows.filter((c) => c.id === fx.quiet_channel_id) });
  });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.quiet_channel_id);
  await expect(page.locator(".lane").first()).toBeVisible();
  await expect(page.getByText(/No tasks in this channel yet|All clear|Start a task/).first()).toBeVisible();
});
