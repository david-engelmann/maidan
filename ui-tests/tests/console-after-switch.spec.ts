import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";
import { laidOutBox } from "./_layout";

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
    const search = page.getByRole("button", { name: /Search and commands/ });
    await expect(search).toBeVisible();
    await expect(search).toContainText("Search");
    const searchBox = await laidOutBox(search);
    expect(searchBox.height).toBeGreaterThanOrEqual(44);
    expect(searchBox.x + searchBox.width).toBeLessThanOrEqual(391);
    // The word sits inside the button, so a grid that stacks the icon over
    // the label cannot clip "Search" out of the 44px control.
    const word = await laidOutBox(search.locator(".btn-label"));
    expect(word.height).toBeGreaterThanOrEqual(10);
    expect(word.y).toBeGreaterThanOrEqual(searchBox.y - 1);
    expect(word.y + word.height).toBeLessThanOrEqual(searchBox.y + searchBox.height + 1);
    const labels = page.locator(".tabbar .label");
    expect(await labels.count()).toBeGreaterThanOrEqual(3);
    for (const label of await labels.all()) {
      expect((await label.textContent())?.trim() ?? "").not.toBe("");
      const box = await laidOutBox(label);
      expect(box.x).toBeGreaterThanOrEqual(-1);
      expect(box.x + box.width).toBeLessThanOrEqual(391);
    }
    await page.locator(".tabbar a", { hasText: "Board" }).click();
    await expect(page.locator("header.top h1")).toHaveText(/Board/);
  });
});

test("an empty Needs you is calm: one quiet card, no buttons", async ({ page }) => {
  await page.route(/\/members\/[^/]+\/waiting/, (route) => route.fulfill({ json: [] }));
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/needs");
  const empty = page.locator(".needs-empty");
  await expect(empty).toContainText("All caught up");
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
  // Only the open row's actions are on screen; the first match in DOM order
  // can be a closed row's, which cannot take focus, and rows waiting the same
  // time have no fixed order.
  const first = page.locator(".need-actions button:not([disabled]):visible").first();
  test.skip((await first.count()) === 0, "nothing waiting in this seed");
  // Reach it with Tab, as a keyboard user does. A script `focus()` with no
  // keyboard input before it does not match `:focus-visible` in Chromium, so
  // the ring never drew and the check failed on every run.
  const handle = await first.elementHandle();
  let reached = false;
  for (let presses = 0; presses < 200 && !reached; presses++) {
    await page.keyboard.press("Tab");
    reached = await page.evaluate((el) => document.activeElement === el, handle);
  }
  expect(reached, "Tab never reached the row's first action").toBe(true);
  // Rows keep loading after the count shows (Add a note enables once the
  // evidence arrives). Focus can move a moment after the redraw, and it must
  // not settle on a disabled control. Poll until it has landed.
  await expect.poll(() =>
    page.evaluate(() => {
      const el = document.activeElement;
      return el instanceof HTMLButtonElement && Boolean(el.closest(".need-actions")) && el.disabled !== true;
    }),
  ).toBe(true);
  const focused = page.locator(".need-actions button:focus");
  await expect(focused).toHaveCount(1);
  await expect(focused).toHaveCSS("outline-style", "solid");
});

test("the palette opens on the keyboard and goes where Enter says", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id);
  await expect(page.locator(".lane").first()).toBeVisible();
  const channel = page.locator(".channel-pick select");
  await expect(channel).toBeEnabled();
  await expect.poll(() => channel.evaluate((el: HTMLSelectElement) => el.options.length)).toBeGreaterThan(1);
  await page.keyboard.press("ControlOrMeta+k");
  const box = page.getByRole("dialog");
  await expect(box).toBeVisible();
  await expect(box.getByRole("combobox")).toHaveAttribute("placeholder", /Search tasks, messages, people/);
  await page.keyboard.type("people");
  await expect(box.getByRole("option").first()).toContainText(/People/);
  await page.keyboard.press("Enter");
  await expect(box).toBeHidden();
  await expect(page).toHaveURL(/#\/people/);
});

test("the palette jumps to a channel and opens a task from its title", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id);
  await expect(page.locator(".channel-pick select")).toHaveValue(fx.board_channel_id);
  await page.keyboard.press("ControlOrMeta+k");
  const box = page.getByRole("dialog");
  await expect(box).toBeVisible();
  await box.getByRole("combobox").fill("quiet");
  const channel = box.getByRole("option", { name: "#quiet" });
  await expect(channel).toBeVisible();
  await channel.click();
  await expect(box).toBeHidden();
  await expect(page).toHaveURL(new RegExp(`#/board\\?channel=${fx.quiet_channel_id}$`));
  await expect(page.locator(".channel-pick select")).toHaveValue(fx.quiet_channel_id);

  await page.keyboard.press("ControlOrMeta+k");
  await expect(box).toBeVisible();
  await box.getByRole("combobox").fill("#build");
  await expect(box.getByRole("option", { name: "#build", exact: true })).toBeVisible();
  await page.keyboard.press("Escape");

  await page.evaluate((id) => window.localStorage.setItem("maidan_console_channel", id), fx.board_channel_id);
  await page.goto("/ui/#/threads");
  await expect(page.locator(".conv-list .channel-pick select")).toHaveValue(fx.board_channel_id);
  await page.keyboard.press("ControlOrMeta+k");
  await box.getByRole("combobox").fill("quiet");
  await box.getByRole("option", { name: "#quiet" }).click();
  await expect(page).toHaveURL(/#\/threads$/);
  await expect(page.locator(".conv-list .channel-pick select")).toHaveValue(fx.quiet_channel_id);

  await page.keyboard.press("ControlOrMeta+k");
  await box.getByRole("combobox").fill("Deploy v9");
  const task = box.getByRole("option", { name: /Deploy v9/ });
  await expect(task).toBeVisible();
  await task.click();
  await expect(page).toHaveURL(new RegExp(`#/threads/t/${fx.thread_id}$`));
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

test("search filters by a person and follows offer a channel by name", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/tools/search");
  const author = page.getByLabel("Author");
  await expect(author).toBeVisible();
  await expect(author.locator("option", { hasText: "Anyone" })).toHaveCount(1);
  await expect(author.locator("option", { hasText: /Deployer/ })).toHaveCount(1);
  await expect(page.getByLabel("Author id")).toHaveCount(0);

  await page.goto("/ui/#/tools/prefs");
  const channel = page.getByRole("combobox", { name: "Channel" });
  await expect(channel).toBeVisible();
  await expect(channel.locator("option", { hasText: "#general" })).toHaveCount(1);
  await expect(channel.locator("option", { hasText: "#quiet" })).toHaveCount(1);
  await expect(page.getByLabel("Channel id")).toHaveCount(0);
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
  const only = page.locator(".channel-only");
  await expect(only).toBeVisible();
  await expect(only).toHaveAttribute("aria-label", "Channel quiet");
  await expect(only).toContainText("quiet");
  await expect(page.locator(".channel-pick select")).toHaveCount(0);
  await expect(page.locator(".channel-pick svg")).toHaveCount(0);
  await page.goto("/ui/#/threads");
  await expect(page.locator(".conv-list .channel-only")).toContainText("quiet");
  await expect(page.locator(".channel-pick select")).toHaveCount(0);
});
