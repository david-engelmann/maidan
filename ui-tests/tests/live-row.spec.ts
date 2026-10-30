import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

async function centers(page: Page) {
  return page.evaluate(() =>
    ["#ws-status", "#ws-connect", "#live-more > summary"].map((s) => {
      // Measure the words, not the box: a tall box can still hold its text
      // at the top.
      const el = document.querySelector(s)!;
      const r = document.createRange();
      const text = [...el.childNodes].find((n) => n.nodeType === Node.TEXT_NODE && n.textContent!.trim());
      r.selectNodeContents(text ?? el);
      const b = r.getBoundingClientRect();
      return b.top + b.height / 2;
    }),
  );
}

// The status, its button and the Live menu read as one line: "Live" used to
// ride about 12px above the other two on a phone.
for (const touch of [false, true]) {
  for (const width of [390, 1440]) {
    test(`the live row's three items share a centre line (${width}, touch=${touch})`, async ({ browser }) => {
      const ctx = await browser.newContext({ viewport: { width, height: 900 }, hasTouch: touch, isMobile: touch && width < 720 });
      const page = await ctx.newPage();
      await signIn(page, fx.workspace_id, fx.review_token);
      await expect(page.locator("#ws-connect")).toBeVisible();
      const c = await centers(page);
      console.log("CENTERS", width, touch, c.map((n) => n.toFixed(1)).join(","));
      expect(Math.max(...c) - Math.min(...c)).toBeLessThanOrEqual(2);
      await ctx.close();
    });
  }
}

test("the live button says Reconnect, not WebSocket jargon", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.review_token);
  await expect(page.getByRole("button", { name: "Reconnect", exact: true })).toBeVisible();
  await expect(page.locator("body")).not.toContainText("Connect WS");
});
