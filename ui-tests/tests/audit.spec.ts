import { test, expect, Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";

// The audit of the board UI: accessibility (axe, keyboard only, reduced
// motion), hostile data, a dropped socket, phone width, and the loading and
// error states. Each finding it was written for is named in its test.
const fx = fixtures();

async function signIn(page: Page, token = fx.review_token, channel = fx.board_channel_id) {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", token);
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator(`#channel-list li[data-id="${channel}"]`)).toBeVisible();
  await page.click(`#channel-list li[data-id="${channel}"]`);
  await expect(page.locator("#board .board-col").first()).toBeVisible();
}

async function expectNoAxeViolations(page: Page, where: string) {
  const result = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  const found = result.violations.map(
    (v) => `${v.id} (${v.impact}) at ${v.nodes.slice(0, 4).map((n) => n.target.join(" ")).join(" | ")}`,
  );
  expect(found, `axe on ${where}`).toEqual([]);
}

for (const vp of [
  { name: "desktop", width: 1280, height: 800 },
  { name: "phone", width: 390, height: 844 },
]) {
  test(`axe finds nothing on the board, Needs you, a thread, the palette and Connect (${vp.name})`, async ({ page }) => {
    await page.setViewportSize({ width: vp.width, height: vp.height });
    await page.goto("/ui/");
    await expectNoAxeViolations(page, `${vp.name}: first visit`);
    await signIn(page);
    await expect(page.locator("#needs-you-list .ny-item").first()).toBeVisible();
    await expect(page.locator("#team .mate").first()).toBeVisible();
    await expectNoAxeViolations(page, `${vp.name}: board and Needs you`);
    await page.locator(`#board .card[data-id="${fx.board_review_thread_id}"]`).click();
    await expect(page.locator("#thread-context")).toHaveText("Review: result waiting on a reviewer");
    await expectNoAxeViolations(page, `${vp.name}: thread`);
    await page.keyboard.press("ControlOrMeta+k");
    await expect(page.locator("#palette")).toBeVisible();
    await expectNoAxeViolations(page, `${vp.name}: palette`);
    await page.keyboard.press("Escape");
    await page.click("#connect-open");
    await expect(page.locator("#connect-dialog")).toBeVisible();
    await expectNoAxeViolations(page, `${vp.name}: Connect an agent`);
  });
}

// Channels and thread rows only answered clicks. Now every row that acts is
// a focusable button with a visible focus ring, so the whole loop works from
// the keyboard: pick a channel, pass the Needs you buttons, open a task.
test("keyboard only: pick a channel, reach Approve, open a task, with a visible focus ring", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.review_token);
  await page.locator("#token").dispatchEvent("change");
  const channel = page.locator(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await expect(channel).toBeVisible();

  const tabUntil = async (pred: string, max = 80): Promise<string[]> => {
    const seen: string[] = [];
    for (let i = 0; i < max; i++) {
      await page.keyboard.press("Tab");
      const hit = await page.evaluate((sel) => {
        const el = document.activeElement as HTMLElement | null;
        return { match: !!el && el.matches(sel), text: el ? (el.textContent || "").trim().slice(0, 40) : "" };
      }, pred);
      seen.push(hit.text);
      if (hit.match) return seen;
    }
    throw new Error(`never reached ${pred}; saw ${seen.join(" / ")}`);
  };

  await tabUntil(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await page.keyboard.press("Enter");
  await expect(page.locator("#board-title")).toHaveText("# build");
  await expect(page.locator("#needs-you-list .ny-item").first()).toBeVisible();

  const path = await tabUntil(`#board .card[data-id="${fx.board_review_thread_id}"]`, 120);
  expect(path, "Approve in Needs you is on the way").toContain("Approve");
  const ring = await page.evaluate(() => getComputedStyle(document.activeElement as Element).outlineStyle);
  expect(ring).not.toBe("none");
  await page.keyboard.press("Enter");
  await expect(page.locator("#thread-context")).toHaveText("Review: result waiting on a reviewer");
});

// Under prefers-reduced-motion nothing animates: no pulsing live dots, no
// gliding cards, no fading rows.
test("reduced motion: the live board runs no animations", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await signIn(page, fx.review_token, fx.floor_channel_id);
  await expect(page.locator("#team .mate").first()).toBeVisible();
  await page.waitForTimeout(600);
  const running = await page.evaluate(() =>
    document.getAnimations().filter((a) => a.playState === "running").map((a) => (a as CSSAnimation).animationName || a.constructor.name),
  );
  expect(running).toEqual([]);
});

// Markup and script URLs in a member name, a channel topic, a task title, a
// message, a result and a gate prompt all render as text, everywhere the
// board shows them.
test("hostile data renders as text in the board, Needs you, team, palette and thread", async ({ page }) => {
  await signIn(page, fx.review_token, fx.lab_channel_id);
  const card = page.locator(`#board .card[data-id="${fx.lab_thread_id}"]`);
  await expect(card).toContainText("Title <img src=x");
  await expect(page.locator("#needs-you-list")).toContainText("Prompt <img src=x");
  await expect(page.locator("#team")).toContainText("Mallory <img");
  await page.keyboard.press("ControlOrMeta+k");
  await page.keyboard.type("Title");
  await expect(page.locator("#palette-list")).toContainText("<script>");
  await page.keyboard.press("Escape");
  await card.click();
  await expect(page.locator("#message-list")).toContainText("Body <img src=x");
  await expect(page.locator("#thread-facts")).toContainText("value <img src=x");

  const report = await page.evaluate(() => ({
    fired: (window as unknown as { __xss?: number }).__xss ?? 0,
    imgs: document.querySelectorAll('img[src="x"]').length,
    scripts: [...document.querySelectorAll("script")].filter((s) => s.textContent?.includes("__xss")).length,
    badLinks: [...document.querySelectorAll("a[href]")]
      .map((a) => (a.getAttribute("href") || "").trim().toLowerCase())
      .filter((h) => h.startsWith("javascript:") || h.startsWith("data:")),
    handlers: [...document.querySelectorAll("*")].filter((el) => [...el.attributes].some((a) => a.name === "onerror")).length,
  }));
  expect(report).toEqual({ fired: 0, imgs: 0, scripts: 0, badLinks: [], handlers: 0 });
});

// The socket reconnected once, 1.5 s after a drop, and gave up if that one
// try failed. Now it backs off and keeps trying, polls the board meanwhile,
// and reconnects at once when the browser comes back online.
test("a dropped socket keeps retrying, the board keeps refreshing, and it comes back", async ({ page, request }) => {
  test.setTimeout(90_000);
  let allow = true;
  let attempts = 0;
  const live: { close: (o?: { code?: number; reason?: string }) => Promise<void> }[] = [];
  await page.routeWebSocket(/\/ws\/subscribe/, (ws) => {
    attempts += 1;
    if (!allow) {
      ws.close({ code: 1011, reason: "down" });
      return;
    }
    ws.connectToServer();
    live.push(ws);
  });
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.live_token);
  await page.locator("#token").dispatchEvent("change");
  await page.click(`#channel-list li[data-id="${fx.lab_channel_id}"]`);
  await expect(page.locator("#board .board-col").first()).toBeVisible();
  await page.click("#ws-connect");
  await expect(page.locator("#ws-status")).toHaveText("connected");

  allow = false;
  await live[live.length - 1].close({ code: 1011, reason: "server restarting" });
  await expect(page.locator("#ws-status")).toContainText("reconnecting in");
  await expect.poll(() => attempts, { timeout: 15_000 }).toBeGreaterThanOrEqual(3);
  await expect(page.locator("#ws-status")).toContainText("reconnecting in");

  // While the socket is down the board still picks up new work by polling.
  const created = await request.post(`${fx.base_url}/channels/${fx.lab_channel_id}/threads`, {
    headers: { Authorization: `Bearer ${fx.token}` },
    data: { title: "Arrived while the socket was down" },
  });
  expect(created.ok()).toBeTruthy();
  await expect(page.locator("#board")).toContainText("Arrived while the socket was down", { timeout: 20_000 });

  allow = true;
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect(page.locator("#ws-status")).toHaveText("connected");
});

// At phone width the page is one column: nothing scrolls sideways, and a
// Needs you row keeps its buttons on screen under its text.
test("phone width: one column, no sideways scroll, Needs you buttons on screen", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await signIn(page);
  const approve = page.locator("#needs-you-list .ny-item").first().getByRole("button", { name: "Approve" });
  await expect(approve).toBeVisible();
  const box = await approve.boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(390);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(overflow).toBeLessThanOrEqual(0);
  const cols = await page.evaluate(() => getComputedStyle(document.getElementById("board")!).gridTemplateColumns.split(" ").length);
  expect(cols).toBe(1);
});

// A picked channel whose tasks fail to load used to keep showing the sign-in
// help, with "TypeError: Failed to fetch" in the sidebar. The board now says
// it is loading, then what failed in words, with Try again.
test("the board shows loading, then a readable error with Try again", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.review_token);
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator(`#channel-list li[data-id="${fx.board_channel_id}"]`)).toBeVisible();

  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  let mode: "slow" | "down" | "ok" = "slow";
  await page.route("**/channels/*/threads?**", async (route) => {
    if (mode === "slow") {
      await gate;
      return route.abort("connectionrefused");
    }
    if (mode === "down") return route.abort("connectionrefused");
    return route.continue();
  });
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  const board = page.locator("#board");
  await expect(board).toContainText("Loading #build");
  await expect(board).toHaveAttribute("aria-busy", "true");
  await expect(board).not.toContainText("sign in above");
  mode = "down";
  release();
  const err = page.locator("#board .board-error");
  await expect(err).toContainText("Could not load #build");
  await expect(err).toContainText("Could not reach the server at");
  await expect(page.locator("#thread-list")).not.toContainText("TypeError");
  mode = "ok";
  await err.getByRole("button", { name: "Try again" }).click();
  await expect(page.locator(`#board .card[data-id="${fx.board_review_thread_id}"]`)).toBeVisible();
});

// Pasting a token signs you in (channels load, the identity pill appears), and
// a bad one says why in words instead of leaving "Not signed in".
test("pasting a token signs in, and a bad token says why", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", "not-a-real-token");
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator("#session-status")).toContainText("That token was not accepted");

  await page.fill("#token", fx.review_token);
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator(`#channel-list li[data-id="${fx.board_channel_id}"]`)).toBeVisible();
  await expect(page.locator("#identity-pill")).toBeVisible();
  await expect(page.locator("#identity-who")).toContainText("Operator");
});

// Switching channel drops the previous task's buttons: they act on a task
// that is no longer on screen.
test("switching channel clears the previous task's header and actions", async ({ page }) => {
  await signIn(page, fx.review_token, fx.desk_channel_id);
  await page.click(`#board .card[data-id="${fx.desk_waiting_thread_id}"]`);
  await expect(page.locator("#thread-actions button").first()).toBeVisible();
  await page.click(`#channel-list li[data-id="${fx.quiet_channel_id}"]`);
  await expect(page.locator("#thread-context")).toHaveText("none selected");
  await expect(page.locator("#thread-actions button")).toHaveCount(0);
  await expect(page.locator("#thread-facts")).toBeEmpty();
});
