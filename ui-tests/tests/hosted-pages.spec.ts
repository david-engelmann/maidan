import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { problemPage } from "../../deploy/cloudflare/src/edge.js";
import { renderStatus, STATUS_CSS, summarize } from "../../deploy/cloudflare/src/status.js";
import { fixtures } from "./_fixtures";

// The Worker serves these pages on the public hosts. The UI test server does
// not, so this spec answers with the same files and renderers on its origin.
const fx = fixtures();
const SITE = resolve(__dirname, "../../deploy/cloudflare/site");
const TRY = resolve(__dirname, "../../deploy/cloudflare/try");
const minted = {
  agent: { token: "mdn_agent_try" },
  admin: { token: "mdn_admin_try" },
  mcp_url: "https://try.maidan.world/mcp/streamable",
};

function text(path: string): string {
  return readFileSync(path, "utf8");
}

async function serve(page: Page, files: Record<string, { body: string; type: string }>): Promise<void> {
  await page.route("**/*", (route) => {
    const path = new URL(route.request().url()).pathname;
    const file = files[path];
    if (!file) return route.continue();
    return route.fulfill({ status: 200, contentType: file.type, body: file.body });
  });
}

async function sideways(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
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

function tryFiles(): Record<string, { body: string; type: string }> {
  return {
    "/": { body: text(resolve(TRY, "index.html")), type: "text/html; charset=utf-8" },
    "/try.css": { body: text(resolve(TRY, "try.css")), type: "text/css; charset=utf-8" },
    "/try.js": { body: text(resolve(TRY, "try.js")), type: "text/javascript; charset=utf-8" },
    "/clients.js": { body: text(resolve(TRY, "clients.js")), type: "text/javascript; charset=utf-8" },
  };
}

test("a sandbox mint reaches the ready state once, and both states pass axe", async ({ page }) => {
  await serve(page, tryFiles());
  await page.route("**/sandbox", (route) => route.fulfill({ status: 201, contentType: "application/json", body: JSON.stringify(minted) }));
  await page.route("**/auth/session/from-token", (route) => route.fulfill({ status: 204, body: "" }));
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto(`${fx.base_url}/`);
  await expect(page.getByRole("heading", { name: /Try Maidan/ })).toBeVisible();
  expect(await page.locator("script[src*='challenges.cloudflare.com']").count()).toBe(0);
  let scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);

  await page.locator("#start").click();
  await expect(page.locator("#ready")).toBeVisible();
  await expect(page.locator("#ready-title")).toBeFocused();
  await expect(page.locator(".notice:visible")).toHaveCount(1);
  await expect(page.locator("#codex-config")).toContainText("tool_timeout_sec = 330");
  await page.locator("button.copy").first().click();
  await expect(page.locator("button.copy").first()).toHaveText("Copied");
  scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
  const signIn = page.waitForRequest((req) => req.url().endsWith("/auth/session/from-token") && req.method() === "POST");
  await page.route("**/ui/**", (route) => route.fulfill({ status: 200, contentType: "text/html", body: "<!doctype html><title>console</title>" }));
  await page.locator("#console").click();
  const posted = await signIn;
  expect(posted.headers().authorization).toBe("Bearer mdn_admin_try");
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("the try page does not scroll sideways and its controls are at least 44px", async ({ page }) => {
    await serve(page, tryFiles());
    await page.goto(`${fx.base_url}/`);
    await expect(page.locator("#start")).toBeVisible();
    expect(await sideways(page)).toBeLessThanOrEqual(1);
    expect(await smallTargets(page, ".brand, .top-link, .cta, .copy, .foot a, .skip")).toEqual([]);
  });
});

const SITE_PAGES = ["privacy.html", "problems.html", "support.html", "terms.html", "not-found.html", "unavailable.html"];

test("the hosted pages are dark by default, have a working skip link, and pass axe in dark and light", async ({ page }) => {
  const files: Record<string, { body: string; type: string }> = {
    "/site.css": { body: text(resolve(SITE, "site.css")), type: "text/css; charset=utf-8" },
  };
  for (const name of SITE_PAGES) {
    files[`/${name.replace(".html", "")}`] = { body: text(resolve(SITE, name)), type: "text/html; charset=utf-8" };
  }
  files["/problem"] = {
    body: problemPage({ title: "Forbidden", detail: "this invite is for another email address" }, "req-123"),
    type: "text/html; charset=utf-8",
  };
  await serve(page, files);
  for (const name of [...SITE_PAGES.map((n) => `/${n.replace(".html", "")}`), "/problem"]) {
    await page.goto(`${fx.base_url}${name}`);
    await expect(page.locator("main#content")).toBeVisible();
    await expect(page.locator('meta[name="color-scheme"]')).toHaveAttribute("content", "dark");
    await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute("content", "#0c0d0f");
    await expect(page.locator("body")).toHaveCSS("color", "rgb(236, 235, 231)");
    await page.locator(".skip").focus();
    await page.keyboard.press("Enter");
    await expect(page.locator("main#content")).toBeFocused();
    const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    // The unavailable page reloads itself on purpose. That refresh is the retry.
    const violations = scan.violations.filter((v) => !(name === "/unavailable" && v.id === "meta-refresh"));
    expect(violations.map((v) => `${name} ${v.id}: ${v.help}`)).toEqual([]);
    // The light palette a page opts into with data-theme passes as well.
    await page.evaluate(() => document.documentElement.setAttribute("data-theme", "light"));
    await expect(page.locator("body")).toHaveCSS("color", "rgb(21, 20, 17)");
    const light = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(light.violations.filter((v) => !(name === "/unavailable" && v.id === "meta-refresh")).map((v) => `${name} light ${v.id}: ${v.help}`)).toEqual([]);
  }
});

test("a problem row clears the sticky header", async ({ page }) => {
  await serve(page, {
    "/problems": { body: text(resolve(SITE, "problems.html")), type: "text/html; charset=utf-8" },
    "/site.css": { body: text(resolve(SITE, "site.css")), type: "text/css; charset=utf-8" },
  });
  await page.goto(`${fx.base_url}/problems#claim-lost`);
  const header = page.locator("header.top");
  const row = page.locator("#claim-lost");
  await expect(row).toBeVisible();
  const head = await header.boundingBox();
  const box = await row.boundingBox();
  expect(head).not.toBeNull();
  expect(box).not.toBeNull();
  expect(box!.y).toBeGreaterThanOrEqual(head!.y + head!.height - 1);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("a hosted page does not scroll sideways and its header and footer are at least 44px", async ({ page }) => {
    await serve(page, {
      "/support": { body: text(resolve(SITE, "support.html")), type: "text/html; charset=utf-8" },
      "/site.css": { body: text(resolve(SITE, "site.css")), type: "text/css; charset=utf-8" },
    });
    await page.goto(`${fx.base_url}/support`);
    await expect(page.locator("main#content")).toBeVisible();
    expect(await sideways(page)).toBeLessThanOrEqual(1);
    expect(await smallTargets(page, ".brand, .signin, .legal a")).toEqual([]);
  });
});

for (const scheme of ["light", "dark"] as const) {
  test(`status passes axe in ${scheme}`, async ({ page }) => {
    const summary = summarize({ status: "ok", db: "ok", storage: "ok", bus: "ok", indexer: "ok", version: "1.2.3" });
    await serve(page, {
      "/status": { body: renderStatus(summary, new Date("2026-11-06T00:00:00Z")), type: "text/html; charset=utf-8" },
      "/status.css": { body: STATUS_CSS, type: "text/css; charset=utf-8" },
    });
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto(`${fx.base_url}/status`);
    await expect(page.getByRole("heading", { name: "Maidan status" })).toBeVisible();
    const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
  });
}

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("status does not scroll sideways", async ({ page }) => {
    const summary = summarize({ status: "ok", db: "ok", storage: "ok", bus: "ok", indexer: "ok", version: "1.2.3" });
    await serve(page, {
      "/status": { body: renderStatus(summary), type: "text/html; charset=utf-8" },
      "/status.css": { body: STATUS_CSS, type: "text/css; charset=utf-8" },
    });
    await page.goto(`${fx.base_url}/status`);
    await expect(page.getByRole("heading", { name: "Maidan status" })).toBeVisible();
    expect(await sideways(page)).toBeLessThanOrEqual(1);
  });
});
