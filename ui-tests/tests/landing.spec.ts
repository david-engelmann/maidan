import { test, expect, type Locator, type Page } from "@playwright/test";
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

// The room is a looping picture whose words fade in and out as the board
// changes, so a scan that lands mid-fade reads a word below 4.5:1. Text in a
// picture is exempt (WCAG 1.4.3), so the scan skips the drawn stage; its
// role="img" wrapper, and the name it carries, are still checked.
function axe(page: Page): AxeBuilder {
  return new AxeBuilder({ page }).exclude(".room-stage__art .ss-stage");
}

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

test("with open sign-up the first paint says Create a workspace, and the invite sheet's code never loads", async ({ page }) => {
  const fetched: string[] = [];
  page.on("request", (req) => fetched.push(new URL(req.url()).pathname));
  await page.route((url) => url.pathname === "/", async (route) => {
    const { readFileSync } = await import("node:fs");
    const html = readFileSync(resolve(DIST, "index.html"), "utf8").replace(
      '<meta name="maidan-signup" content="closed" />',
      '<meta name="maidan-signup" content="open" />',
    );
    return route.fulfill({ body: html, contentType: "text/html; charset=utf-8" });
  });
  await page.route((url) => url.pathname.startsWith("/landing/"), (route) =>
    route.fulfill({ path: resolve(DIST, basename(new URL(route.request().url()).pathname)) }),
  );
  await page.goto(`${fx.base_url}/`);
  await expect(page.getByRole("button", { name: "Create a workspace" }).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Join the waitlist" })).toHaveCount(0);
  await page.waitForLoadState("networkidle");
  expect(fetched.filter((p) => p.endsWith("/motion.js") || p.endsWith("/join.js"))).toEqual([]);
});

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
  await expect(page.getByRole("heading", { name: /shared workspace/ })).toBeVisible();
  expect(await page.locator("script[src*='challenges.cloudflare.com']").count()).toBe(0);
  const scan = await axe(page).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
});

test("the landing is dark by default and takes the console's stored Light", async ({ page }) => {
  // /ui/theme.js is the console's own boot script, so one stored choice
  // themes both, set before the first paint.
  await openLanding(page);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(page.locator("body")).toHaveCSS("color", "rgb(236, 235, 231)");
  await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute("content", "#0c0d0f");
  await page.evaluate(() => window.localStorage.setItem("maidan_console_theme", "light"));
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.locator("body")).toHaveCSS("color", "rgb(21, 20, 17)");
  const scan = await axe(page).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(scan.violations.map((v) => `light ${v.id}: ${v.help}`)).toEqual([]);
  await page.evaluate(() => window.localStorage.removeItem("maidan_console_theme"));
});

// The footer's switch writes the console's own choice, so it themes both,
// and the recording is the console filmed in the page's theme.
test("the footer's theme switch themes the page and its recording, and the choice is kept", async ({ page }) => {
  await openLanding(page);
  const html = page.locator("html");
  const clip = page.locator(".watch-frame source");
  const theme = page.getByRole("group", { name: "Theme" });
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(theme.getByRole("button", { name: "Dark" })).toHaveAttribute("aria-pressed", "true");
  await expect(clip).toHaveAttribute("srcset", "/landing/board-live.avif");

  await theme.getByRole("button", { name: "Light" }).click();
  await expect(html).toHaveAttribute("data-theme", "light");
  await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute("content", "#f2eee6");
  await expect(clip).toHaveAttribute("srcset", "/landing/board-live-light.avif");
  // Paused, the still is the light one too
  await page.locator(".watch-frame").getByRole("button", { name: "Pause the recording" }).click();
  await expect(clip).toHaveAttribute("srcset", "/landing/board-live-light-still.avif");
  await expect(page.locator(".watch-clip")).toHaveAttribute("src", "/landing/board-live-light.png");

  await page.reload();
  await expect(html).toHaveAttribute("data-theme", "light");
  await expect(theme.getByRole("button", { name: "Light" })).toHaveAttribute("aria-pressed", "true");
  await page.evaluate(() => window.localStorage.removeItem("maidan_console_theme"));
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

// Each moving picture can be stopped (WCAG 2.2.2). The control is a small
// glyph over the picture's corner; its name says what a press will do, and
// the ring around it still takes a 44px tap. The journey behind the page has
// one too, which travels with the screen (it is checked with the journey,
// below, since it is there only where a GPU draws the scene).
const MOVING = [
  ["room animation", ".room-stage"],
  ["recording", ".watch-frame"],
] as const;

/** The control's box is at most 36px, and a tap 22px from its middle reaches it. */
async function expectSmallControlWithATapRing(pause: Locator, noun: string): Promise<void> {
  const box = await pause.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.width).toBeLessThanOrEqual(36);
  expect(box!.height).toBeLessThanOrEqual(36);
  const taps = await pause.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    return [[-22, 0], [22, 0], [0, -22], [0, 22]].map(([dx, dy]) => {
      const at = document.elementFromPoint(x + dx, y + dy);
      return !!at && (at === el || el.contains(at));
    });
  });
  expect(taps, `${noun}: a tap 22px from the glyph's middle reaches it`).toEqual([true, true, true, true]);
}

test("each moving picture has a small pause control that names what a press will do", async ({ page }) => {
  await openLanding(page);
  for (const [noun, frame] of MOVING) {
    const pause = page.locator(frame).getByRole("button", { name: `Pause the ${noun}` });
    await pause.scrollIntoViewIfNeeded();
    await expect(pause).toBeVisible();
    await expect(pause).not.toHaveAttribute("aria-pressed");
    await expectSmallControlWithATapRing(pause, noun);
    await pause.click();
    await expect(page.locator(frame).getByRole("button", { name: `Play the ${noun}` })).toBeVisible();
  }
});

const CAST = ["Claude Code", "Codex", "Cursor", "Grok", "Muse", "Antigravity", "Maya", "Jun"];

test("the room's board loads near its section and plays claim, review and land, lighting each caption in turn", async ({ page }) => {
  test.setTimeout(60_000);
  const fetched: string[] = [];
  page.on("request", (req) => fetched.push(new URL(req.url()).pathname));
  await openLanding(page);
  expect(fetched).not.toContain("/landing/room.js");
  const stage = page.locator(".room-stage");
  await stage.scrollIntoViewIfNeeded();
  await expect.poll(() => fetched.includes("/landing/room.js")).toBe(true);
  // The picture is named as a whole, with everyone on the board
  const art = page.locator(".room-stage__art");
  await expect(art).toHaveAttribute("role", "img");
  for (const who of CAST) {
    await expect(art).toHaveAttribute("aria-label", new RegExp(who));
    await expect(stage.locator(".rm-cur__tag", { hasText: who })).toHaveCount(1);
  }
  const steps = page.locator(".room-step");
  const beats = [
    ["lease 15m", 0],
    ["Codex reviewing", 1],
    ["PR #482 merged", 2],
  ] as const;
  for (const [words, step] of beats) {
    await expect(stage.getByText(words).first()).toBeVisible({ timeout: 25_000 });
    await expect(steps.nth(step)).toHaveAttribute("data-active", "true", { timeout: 25_000 });
  }
  // The send-back and the merge are both on the record
  await expect(stage.locator(".rm-log")).toContainText("PR #482 merged");
});

test.describe("with reduced motion", () => {
  test.use({ reducedMotion: "reduce" });

  test("the room is one still frame of the landed task, with nothing to pause", async ({ page }) => {
    await openLanding(page);
    await page.locator(".room-stage").scrollIntoViewIfNeeded();
    await expect(page.locator(".journey-toggle")).toHaveCount(0);
    await expect(page.locator(".room-stage .media-toggle")).toHaveCount(0);
    await expect(page.locator(".watch-frame").getByRole("button", { name: "Play the recording" })).toBeVisible();
    const stage = page.locator(".room-stage");
    await expect(stage.locator(".rm-card", { hasText: "PR #482 merged" })).toBeVisible();
    await expect(stage.locator(".rm-log")).toContainText("Codex approved Claude Code’s change");
    await expect(page.locator(".room-step[data-active]")).toHaveCount(0);
  });

  test("the journey is a composed still, redrawn as the page scrolls, with nothing to pause", async ({ page }) => {
    await openLanding(page);
    // A still where a GPU draws it; the poster of the light where WebGL is
    // missing or drawn on the CPU
    const journey = page.locator(".journey");
    await expect(journey).toHaveAttribute("data-state", /^(still|software|fallback)$/);
    await page.locator(".how").scrollIntoViewIfNeeded();
    await expect(journey).toHaveAttribute("data-state", /^(still|software|fallback)$/);
    await expect(page.locator(".journey-toggle")).toHaveCount(0);
  });
});

test("the journey behind the page is decoration in its own chunk, and its control stops it", async ({ page }) => {
  const fetched: string[] = [];
  page.on("request", (req) => fetched.push(new URL(req.url()).pathname));
  await openLanding(page);
  // One picture, fixed behind the whole page
  const journey = page.locator(".journey");
  await expect(journey).toHaveCount(1);
  await expect(journey).toHaveAttribute("aria-hidden", "true");
  await expect(journey).toHaveCSS("position", "fixed");
  await expect(journey).toHaveCSS("pointer-events", "none");
  // Its code is its own chunk, asked for once the page has loaded
  await expect.poll(() => fetched.includes("/landing/journey.js")).toBe(true);
  expect(fetched).not.toContain("/landing/gathering.js");
  await expect(journey).toHaveAttribute("data-state", /^(running|software|fallback)$/);
  // Where WebGL2 exists the scene compiles and draws, even on a software
  // renderer that then leaves the poster up
  const webgl = await page.evaluate(() => !!document.createElement("canvas").getContext("webgl2"));
  if (webgl) await expect(journey).not.toHaveAttribute("data-state", "fallback");
  const running = (await journey.getAttribute("data-state")) === "running";
  // The poster holds the hero until the scene draws, and wherever it cannot
  if (running) await expect(page.locator("html")).toHaveAttribute("data-journey", "on");
  else await expect(page.locator("html")).not.toHaveAttribute("data-journey", "on");
  await expect(page.locator(".journey-poster")).toHaveCSS("opacity", running ? "0" : "1");
  test.skip(!running, "no GPU here: the poster stands in for the scene");
  // It travels with the screen: there at the top of the page and at its end
  const pause = page.getByRole("button", { name: "Pause the animation" });
  await expect(pause).toBeVisible();
  // in the header's bar, where no line of copy runs under it
  await expect(page.locator(".top .journey-toggle")).toHaveCount(1);
  await expect(pause).not.toHaveAttribute("aria-pressed");
  await expectSmallControlWithATapRing(pause, "animation");
  await pause.click();
  await expect(journey).toHaveAttribute("data-state", "paused");
  await page.locator(".close").scrollIntoViewIfNeeded();
  const play = page.getByRole("button", { name: "Play the animation" });
  await expect(play).toBeInViewport();
  await play.click();
  await expect(journey).toHaveAttribute("data-state", "running");
});

test("without WebGL the poster of the light stays, and nothing errs", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.addInitScript(() => {
    // A browser with no WebGL: every other context still answers
    const get = HTMLCanvasElement.prototype.getContext as (this: HTMLCanvasElement, ...args: unknown[]) => unknown;
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
      value(this: HTMLCanvasElement, type: string, ...rest: unknown[]) {
        return type === "webgl2" || type === "webgl" ? null : get.call(this, type, ...rest);
      },
    });
  });
  await openLanding(page);
  const journey = page.locator(".journey");
  await expect(journey).toHaveAttribute("data-state", "fallback");
  await expect(journey.locator(".journey__canvas")).toHaveCSS("opacity", "0");
  await expect(page.locator("html")).not.toHaveAttribute("data-journey", "on");
  const poster = page.locator(".journey-poster");
  await expect(poster).toHaveCSS("opacity", "1");
  expect(await poster.evaluate((el) => getComputedStyle(el).backgroundImage)).toContain("data:image/webp");
  // Nothing to pause, and the close ends after its words
  await expect(page.locator(".journey-toggle")).toHaveCount(0);
  expect(errors).toEqual([]);
});

// The picture must never widen the page, from the narrowest phone to a wide desktop.
for (const width of [320, 768, 1024, 1920]) {
  test(`at ${width}px the page stays within the screen`, async ({ page }) => {
    await page.setViewportSize({ width, height: width < 700 ? 640 : 900 });
    await openLanding(page);
    await expect(page.locator(".journey")).toHaveAttribute("data-state", /^(running|software|fallback)$/);
    for (const stop of [".hero", ".how", ".close"]) {
      await page.locator(stop).scrollIntoViewIfNeeded();
      expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth), `${stop} at ${width}px`).toBeLessThanOrEqual(0);
    }
  });
}

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

// A phone's first screen holds the light, the headline and the first call
// to action, the short phone included.
for (const vp of [{ width: 390, height: 844 }, { width: 375, height: 667 }]) {
  test.describe(`on a ${vp.width}x${vp.height} phone`, () => {
    test.use({ viewport: vp, isMobile: true, hasTouch: true });

    test("the headline and the first call to action are on the first screen", async ({ page }) => {
      await openLanding(page);
      const header = await page.locator(".top").boundingBox();
      const eyebrow = await page.locator(".hero .eyebrow").boundingBox();
      const h1 = await page.locator(".hero h1").boundingBox();
      const cta = await page.locator(".hero .cta").first().boundingBox();
      for (const box of [header, eyebrow, h1, cta]) expect(box).not.toBeNull();
      // The words start a little under the header, with air over them
      expect(eyebrow!.y - (header!.y + header!.height)).toBeGreaterThanOrEqual(24);
      // and the poster that stands in for it is the first screen's
      const poster = await page.locator(".journey-poster").boundingBox();
      expect(poster).not.toBeNull();
      expect(poster!.y).toBe(0);
      expect(Math.round(poster!.height)).toBe(vp.height);
      expect(h1!.y + h1!.height).toBeLessThanOrEqual(cta!.y);
      expect(cta!.y + cta!.height).toBeLessThanOrEqual(vp.height);
      await expect(page.locator(".hero .cta").first()).toHaveText(/Join the waitlist|Create a workspace/);
    });
  });
}

test("the waitlist sheet validates, accepts a join, and Done returns to the button", async ({ page }) => {
  await openLanding(page);
  const open = page.locator(".join-btn");
  await open.click();
  const sheet = page.locator("#join");
  await expect(sheet).toHaveAttribute("aria-modal", "true");
  await expect(page.locator("#wl-email")).toBeFocused();
  const scan = await axe(page).withTags(["wcag2a", "wcag2aa"]).analyze();
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
  const scan = await axe(page).withTags(AXE).analyze();
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
      await expect(page.getByRole("heading", { name: /shared workspace/ })).toBeVisible();
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
