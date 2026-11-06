import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { basename, resolve } from "node:path";

// Sign-up is off on the UI test server, so `/` is a route index and
// `/landing/` answers 404. The built page is the one build.rs embeds, served
// here on the same origin. `POST /waitlist` is answered locally.
const LANDING_DIST = resolve(__dirname, "../../crates/maidan-server/static/landing/dist");
const AXE_TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];

async function openLanding(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const clipboard = { writeText: async () => {} };
    try {
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: clipboard });
    } catch {
      // The browser already exposed a clipboard. The page can use that one.
    }
  });
  await page.route((url) => url.pathname === "/", (route) =>
    route.fulfill({ path: resolve(LANDING_DIST, "index.html"), contentType: "text/html; charset=utf-8" }),
  );
  await page.route((url) => url.pathname.startsWith("/landing/"), (route) =>
    route.fulfill({ path: resolve(LANDING_DIST, basename(new URL(route.request().url()).pathname)) }),
  );
  await page.route("**/waitlist", (route) => {
    if (route.request().method() !== "POST") return route.continue();
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ email: "ada@example.com", status: "waiting" }),
    });
  });
  await page.goto("/");
}

async function axeClean(page: Page): Promise<void> {
  const scan = await new AxeBuilder({ page }).withTags(AXE_TAGS).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
}

async function sectionClearsHeader(page: Page, id: string): Promise<void> {
  await page.evaluate((hash) => {
    location.hash = hash;
  }, id);
  await expect.poll(async () => page.evaluate((sel) => {
    const section = document.querySelector(sel);
    const header = document.querySelector(".top");
    if (!section || !header) return -1;
    const s = section.getBoundingClientRect();
    const h = header.getBoundingClientRect();
    return s.y - (h.y + h.height);
  }, id)).toBeGreaterThanOrEqual(-2);
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

    test("the page stays on the screen and has no accessibility violations", async ({ page }) => {
      await openLanding(page);
      await expect(page.getByRole("heading", { name: /Agents claim/ })).toBeVisible();
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
      expect(overflow).toBeLessThanOrEqual(0);
      expect(errors).toEqual([]);
      await axeClean(page);
    });

    test("the example tabs move with the keyboard and the copy state resets", async ({ page }) => {
      await openLanding(page);
      const claude = page.locator("#dev-tab-claude");
      const python = page.locator("#dev-tab-python");
      const self = page.locator("#dev-tab-self");
      const panel = page.locator("#dev-panel");
      const copy = page.locator(".copy");
      await claude.focus();
      await page.keyboard.press("ArrowRight");
      await expect(python).toBeFocused();
      await expect(python).toHaveAttribute("aria-selected", "true");
      await expect(panel).toContainText("from maidan import Client");
      await page.keyboard.press("ArrowRight");
      await expect(self).toBeFocused();
      await expect(self).toHaveAttribute("aria-selected", "true");
      await expect(panel).toContainText("compose.quickstart.yaml");
      await page.keyboard.press("ArrowRight");
      await expect(claude).toBeFocused();
      await expect(claude).toHaveAttribute("aria-selected", "true");
      await expect(panel).toContainText("claude mcp add");
      await page.keyboard.press("End");
      await expect(self).toBeFocused();
      await page.keyboard.press("Home");
      await expect(claude).toBeFocused();

      await copy.click();
      await expect(copy).toHaveText("Copied");
      await expect(page.locator(".run-card [role=status]")).toContainText("Copied");
      await python.click();
      await expect(copy).toHaveText("Copy");
    });

    test("section anchors clear the fixed header", async ({ page }) => {
      await openLanding(page);
      await sectionClearsHeader(page, "#developers");
      await sectionClearsHeader(page, "#how");
    });

    test("the waitlist sheet validates, confirms, and returns focus", async ({ page }) => {
      await openLanding(page);
      const opener = page.getByRole("button", { name: "Join the waitlist" }).first();
      await opener.click();
      const sheet = page.locator("#join");
      await expect(sheet).toHaveAttribute("aria-modal", "true");
      const close = page.locator(".sheet-close");
      const box = await close.boundingBox();
      expect(box).not.toBeNull();
      expect(box!.width).toBeGreaterThanOrEqual(44);
      expect(box!.height).toBeGreaterThanOrEqual(44);

      const email = page.locator("#wl-email");
      await expect(email).toBeFocused();
      await close.focus();
      await page.keyboard.press("Shift+Tab");
      await expect(page.locator("#wl-submit")).toBeFocused();
      await page.keyboard.press("Tab");
      await expect(close).toBeFocused();
      await axeClean(page);

      await page.locator("#wl-submit").click();
      await expect(email).toBeFocused();
      for (const id of ["wl-email", "wl-use", "wl-consent"]) {
        const field = page.locator(`#${id}`);
        await expect(field).toHaveAttribute("aria-invalid", "true");
        await expect(field).toHaveAttribute("aria-describedby", `${id}-err`);
      }
      await axeClean(page);

      await email.fill("ada@example.com");
      await page.locator("#wl-use").fill("A review queue for our agents.");
      await page.locator("#wl-consent").check();
      for (const id of ["wl-email", "wl-use", "wl-consent"]) {
        await expect(page.locator(`#${id}`)).not.toHaveAttribute("aria-describedby");
      }

      await page.locator("#wl-submit").click();
      await expect(page.getByRole("heading", { name: "You're on the list." })).toBeVisible();
      await expect(page.locator(".done")).toBeFocused();
      await axeClean(page);
      await page.keyboard.press("Escape");
      await expect(sheet).toHaveAttribute("aria-modal", "false");
      await expect(opener).toBeFocused();
    });
  });
}
