import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { basename, resolve } from "node:path";
import { fixtures } from "./_fixtures";

// Sign-up is off on the UI test server, so `/` is a route index and
// `/landing/` answers 404. The built page is served from the files the
// server embed would use, on the same origin, and the form is answered here
// because `POST /waitlist` is not enabled.
const fx = fixtures();
const DIST = resolve(__dirname, "../../crates/maidan-server/static/landing/dist");

const AXE = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

async function openLanding(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const clipboard = { writeText: async () => {} };
    try {
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: clipboard });
    } catch {
      // The browser already exposed a clipboard.
    }
  });
  await page.route((url) => url.pathname === "/", (route) =>
    route.fulfill({ path: resolve(DIST, "index.html"), contentType: "text/html; charset=utf-8" }),
  );
  await page.route((url) => url.pathname.startsWith("/landing/"), (route) =>
    route.fulfill({ path: resolve(DIST, basename(new URL(route.request().url()).pathname)) }),
  );
  await page.route("**/waitlist", (route) => {
    if (route.request().method() !== "POST") return route.continue();
    return route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({ email: "ada@example.com", status: "waiting" }),
    });
  });
  await page.goto(`${fx.base_url}/`);
}

async function smallTargets(page: Page, selector: string): Promise<string[]> {
  return page.locator(selector).evaluateAll((els) =>
    els
      .map((e) => {
        const r = e.getBoundingClientRect();
        return { t: (e.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 24), w: Math.round(r.width), h: Math.round(r.height) };
      })
      .filter(({ w, h }) => w > 0 && h > 0 && (w < 44 || h < 44))
      .map(({ t, w, h }) => `${t} ${w}x${h}`),
  );
}

test("the page loads without the check script and has no serious accessibility gaps", async ({ page }) => {
  await openLanding(page);
  await expect(page.getByRole("heading", { name: /Agents claim/ })).toBeVisible();
  expect(await page.locator("script[src*='challenges.cloudflare.com']").count()).toBe(0);
  const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
});

test("the example tabs move with the keyboard and wrap", async ({ page }) => {
  await openLanding(page);
  const claude = page.locator("#dev-tab-claude");
  const python = page.locator("#dev-tab-python");
  const self = page.locator("#dev-tab-self");
  const panel = page.locator("#dev-panel");
  await claude.focus();
  await page.keyboard.press("ArrowRight");
  await expect(python).toBeFocused();
  await expect(python).toHaveAttribute("aria-selected", "true");
  await expect(panel).toContainText("from maidan import Client");
  await page.keyboard.press("ArrowRight");
  await expect(self).toBeFocused();
  await expect(panel).toContainText("compose.quickstart.yaml");
  await page.keyboard.press("ArrowRight");
  await expect(claude).toBeFocused();
  await expect(panel).toContainText("claude mcp add");
  await page.keyboard.press("End");
  await expect(self).toBeFocused();
  await page.keyboard.press("Home");
  await expect(claude).toBeFocused();
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("the page and the open sheet stay within the screen, and controls are at least 44px", async ({ page }) => {
    await openLanding(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
    await page.locator(".join-btn").click();
    await expect(page.locator("#join")).toHaveAttribute("aria-modal", "true");
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
    const small = await smallTargets(page, ".join-btn, .cta, .tab, .copy, .sheet-close, .submit");
    expect(small).toEqual([]);
  });
});

test("the waitlist sheet validates, accepts a join, and Done returns to the button", async ({ page }) => {
  await openLanding(page);
  const open = page.locator(".join-btn");
  await open.click();
  const sheet = page.locator("#join");
  await expect(sheet).toHaveAttribute("aria-modal", "true");
  await expect(page.locator("#wl-email")).toBeFocused();
  const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);

  await page.keyboard.press("Tab");
  await expect(page.locator("#wl-use")).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(page.locator("#wl-email")).toBeFocused();

  await page.locator("#wl-submit").click();
  await expect(page.locator("#wl-email")).toHaveAttribute("aria-invalid", "true");
  await expect(page.locator("#wl-email")).toBeFocused();

  await page.locator("#wl-email").fill("ada@example.com");
  await page.locator("#wl-use").fill("A review queue for our agents.");
  await page.locator("#wl-consent").check();
  await page.locator("#wl-submit").click();
  await expect(page.getByRole("heading", { name: "You're on the list." })).toBeVisible();
  await page.getByRole("button", { name: "Done" }).click();
  await expect(sheet).toHaveAttribute("aria-modal", "false");
  await expect(open).toBeFocused();
});

async function axeClean(page: Page): Promise<void> {
  const scan = await new AxeBuilder({ page }).withTags(AXE).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
}

const VIEWPORTS = [
  { name: "desktop", width: 1440, height: 900, mobile: false },
  { name: "phone", width: 390, height: 844, mobile: true },
  { name: "narrow phone", width: 360, height: 800, mobile: true },
] as const;

for (const vp of VIEWPORTS) {
  test.describe(vp.name, () => {
    test.use({ viewport: { width: vp.width, height: vp.height }, isMobile: vp.mobile, hasTouch: vp.mobile });

    let errors: string[] = [];
    test.beforeEach(({ page }) => {
      errors = [];
      page.on("pageerror", (err) => errors.push(`pageerror: ${err.message}`));
      page.on("response", (res) => {
        const kind = res.request().resourceType();
        if (res.status() >= 400 && ["stylesheet", "script", "font", "image"].includes(kind)) {
          errors.push(`${kind} ${res.status()}: ${new URL(res.url()).pathname}`);
        }
      });
    });

    test("stays on the screen with no accessibility violations", async ({ page }) => {
      await openLanding(page);
      await expect(page.getByRole("heading", { name: /Agents claim/ })).toBeVisible();
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
      expect(overflow).toBeLessThanOrEqual(0);
      expect(errors).toEqual([]);
      await axeClean(page);
    });

    test("copy resets when the tab changes, and anchors clear the header", async ({ page }) => {
      await openLanding(page);
      const copy = page.locator(".copy");
      await copy.click();
      await expect(copy).toHaveText("Copied");
      await expect(page.locator(".run-card [role=status]")).toContainText("Copied");
      await page.locator("#dev-tab-python").click();
      await expect(copy).toHaveText("Copy");

      for (const id of ["#developers", "#how"]) {
        await page.evaluate((hash) => { location.hash = hash; }, id);
        await expect.poll(async () => page.evaluate((sel) => {
          const section = document.querySelector(sel);
          const header = document.querySelector(".top");
          if (!section || !header) return -1;
          const s = section.getBoundingClientRect();
          const h = header.getBoundingClientRect();
          return s.y - (h.y + h.height);
        }, id)).toBeGreaterThanOrEqual(-2);
      }
    });

    test("errors are named only while they show, and Escape returns focus", async ({ page }) => {
      await openLanding(page);
      const opener = page.getByRole("button", { name: "Join the waitlist" }).first();
      await opener.click();
      const close = page.locator(".sheet-close");
      const box = await close.boundingBox();
      expect(box).not.toBeNull();
      expect(box!.width).toBeGreaterThanOrEqual(44);
      expect(box!.height).toBeGreaterThanOrEqual(44);
      await expect(page.locator("#wl-email")).toBeFocused();
      await close.focus();
      await page.keyboard.press("Shift+Tab");
      await expect(page.locator("#wl-submit")).toBeFocused();
      await page.keyboard.press("Tab");
      await expect(close).toBeFocused();

      await page.locator("#wl-submit").click();
      await expect(page.locator("#wl-email")).toBeFocused();
      for (const id of ["wl-email", "wl-use", "wl-consent"]) {
        const field = page.locator(`#${id}`);
        await expect(field).toHaveAttribute("aria-invalid", "true");
        await expect(field).toHaveAttribute("aria-describedby", `${id}-err`);
      }
      await axeClean(page);
      await page.locator("#wl-email").fill("ada@example.com");
      await page.locator("#wl-use").fill("A review queue for our agents.");
      await page.locator("#wl-consent").check();
      for (const id of ["wl-email", "wl-use", "wl-consent"]) {
        await expect(page.locator(`#${id}`)).not.toHaveAttribute("aria-describedby");
      }
      await page.locator("#wl-submit").click();
      await expect(page.getByRole("heading", { name: "You're on the list." })).toBeVisible();
      await expect(page.locator(".done")).toBeFocused();
      await page.keyboard.press("Escape");
      await expect(page.locator("#join")).toHaveAttribute("aria-modal", "false");
      await expect(opener).toBeFocused();
    });
  });
}
