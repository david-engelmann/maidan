import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// The classic board's keyboard guarantees, held on the console: every control
// is a real button or link (no role=button on a div), a Tab walk never drops
// to the page body, and a live refresh does not throw focus away.
const fx = fixtures();
const SCREENS = ["#/board", "#/needs", "#/threads", "#/people", "#/tools/prefs"];

/** Each Tab stop as its tag plus a short fingerprint of the element. */
async function walk(page: Page, steps: number): Promise<string[]> {
  const stops: string[] = [];
  for (let i = 0; i < steps; i++) {
    await page.keyboard.press("Tab");
    stops.push(await page.evaluate(() => {
      const el = document.activeElement;
      if (!el) return "NONE";
      return el === document.body ? "BODY" : `${el.tagName}:${el.outerHTML.slice(0, 60)}`;
    }));
  }
  return stops;
}

/**
 * Body stops that are not the walk wrapping past the last control back to the
 * top. A wrap lands on the page's first control (the skip link) or on one
 * already seen. That need not be stops[0]: the click that starts the walk sets
 * the navigation start point past the skip link, so only the wrap reaches it.
 */
function holes(stops: string[], top: string): number[] {
  return stops.flatMap((s, i) => {
    if (s === "NONE") return [i];
    if (s !== "BODY" || i + 1 >= stops.length) return [];
    const next = stops[i + 1];
    return next === top || stops.slice(0, i).includes(next) ? [] : [i];
  });
}

/** The fingerprint of the first control in document order, as walk() writes it. */
async function firstControl(page: Page): Promise<string> {
  return page.evaluate(() => {
    const el = document.querySelector("a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex='0']");
    return el ? `${el.tagName}:${el.outerHTML.slice(0, 60)}` : "NONE";
  });
}

for (const hash of SCREENS) {
  test(`${hash}: controls are real buttons, and a Tab walk never lands on the body`, async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id, hash);
    await page.waitForLoadState("networkidle");
    const fake = await page.locator('[role="button"]:not(button):not(a):not(summary)').count();
    expect(fake, "a control drawn as a div with role=button").toBe(0);
    const unnamed = await page.locator("button:visible").evaluateAll((els) =>
      els.filter((b) => !(b.getAttribute("aria-label") || b.textContent?.trim() || b.getAttribute("title"))).map((b) => b.outerHTML.slice(0, 80)),
    );
    expect(unnamed, "buttons with no name").toEqual([]);
    await page.locator("body").click({ position: { x: 1, y: 1 } });
    const stops = await walk(page, 40);
    expect(holes(stops, await firstControl(page)), `stops: ${stops.map((s) => s.split(":")[0]).join(" ")}`).toEqual([]);
  });
}

test("a live refresh keeps focus on the card it was on", async ({ page, request }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/board");
  const card = page.locator(".task").first();
  await expect(card).toBeVisible();
  const name = await card.getAttribute("aria-label");
  await card.focus();
  await expect(card).toBeFocused();
  // A new task lands in the channel: the board re-reads and the lanes shift.
  const reread = page.waitForResponse((r) => /\/channels\/[^/]+\/threads/.test(r.url()) && r.request().method() === "GET", { timeout: 15_000 });
  const res = await request.post(`/channels/${fx.board_channel_id}/threads`, { headers: { Authorization: `Bearer ${fx.token}` }, data: { title: `focus probe ${Date.now()}` } });
  expect(res.ok(), `create: ${res.status()}`).toBeTruthy();
  await reread;
  await page.waitForTimeout(400);
  const now = await page.evaluate(() => ({ tag: document.activeElement?.tagName, name: document.activeElement?.getAttribute("aria-label") ?? null }));
  expect(now.tag, "focus fell to the page").not.toBe("BODY");
  expect(now.name).toBe(name);
});
