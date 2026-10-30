import { mkdirSync, readFileSync, writeFileSync } from "fs";
import { resolve } from "path";
import { test, expect, type Page } from "@playwright/test";

// The review gallery: the screens a person sees first, at a desktop and a
// phone width, so a UI change is looked at before it merges rather than after
// it ships. The pictures are for eyes; two things are checked because no eye
// should have to: a screen that scrolls sideways on its own viewport, and a
// page that throws. Both fail the run, and report.json lists every screen
// with what was measured.

interface Board {
  workspace_id: string;
  channel_id: string;
  token: string;
  review_thread_id: string | null;
}
interface Capture {
  now: string;
  flight: Board;
  waiting: Board;
  empty: Board;
}

const fx: Capture = JSON.parse(readFileSync(resolve(__dirname, "../.gallery.json"), "utf8"));
// The server answers `/` with the landing only where sign-up is on, and the
// capture seed leaves it off. The page is the one build.rs wrote; its assets
// come from the server's /landing/ route as they do in production.
const LANDING = resolve(__dirname, "../../crates/maidan-server/static/landing/dist/index.html");
async function openLanding(page: Page) {
  await page.route((url) => url.pathname === "/", (route) =>
    route.fulfill({ path: LANDING, contentType: "text/html; charset=utf-8" }),
  );
  await page.goto("/", { waitUntil: "networkidle" });
}
const OUT = resolve(__dirname, "..", process.env.GALLERY_OUT ?? "gallery-out");
mkdirSync(OUT, { recursive: true });

const VIEWPORTS = [
  { tag: "desktop", width: 1440, height: 900, mobile: false },
  { tag: "phone", width: 390, height: 844, mobile: true },
] as const;

interface Row {
  shot: string;
  width: number;
  sideways_px: number;
  errors: string[];
}
const rows: Row[] = [];

test.afterAll(() => {
  writeFileSync(resolve(OUT, "report.json"), JSON.stringify(rows, null, 2) + "\n");
});

async function signIn(page: Page, board: Board) {
  await page.clock.setFixedTime(new Date(fx.now));
  await page.goto("/ui/");
  await page.fill("#workspace", board.workspace_id);
  await page.fill("#token", board.token);
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator("#logout")).toBeAttached();
  await page.reload();
  await expect(page.locator("#board")).not.toHaveAttribute("aria-busy", "true");
}

async function shoot(page: Page, name: string, width: number, errors: string[]) {
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  });
  await page.mouse.move(0, 0);
  const sideways = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  await page.screenshot({ path: resolve(OUT, `${name}.png`), fullPage: true, animations: "disabled", caret: "hide" });
  rows.push({ shot: name, width, sideways_px: sideways, errors: [...errors] });
  expect(sideways, `${name} scrolls sideways by ${sideways}px`).toBeLessThanOrEqual(0);
  expect(errors, `${name} logged errors`).toEqual([]);
}

for (const vp of VIEWPORTS) {
  test.describe(vp.tag, () => {
    test.use({ viewport: { width: vp.width, height: vp.height }, isMobile: vp.mobile, hasTouch: vp.mobile });

    let errors: string[] = [];
    test.beforeEach(({ page }) => {
      errors = [];
      page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
    });

    test(`landing, ${vp.tag}`, async ({ page }) => {
      await openLanding(page);
      await shoot(page, `landing-${vp.tag}`, vp.width, errors);
    });

    test(`landing, reduced motion, ${vp.tag}`, async ({ page }) => {
      await page.emulateMedia({ reducedMotion: "reduce" });
      await openLanding(page);
      await shoot(page, `landing-reduced-motion-${vp.tag}`, vp.width, errors);
    });

    test(`console signed out, ${vp.tag}`, async ({ page }) => {
      await page.clock.setFixedTime(new Date(fx.now));
      await page.goto("/ui/");
      await expect(page.locator("#token")).toBeAttached();
      await shoot(page, `console-signed-out-${vp.tag}`, vp.width, errors);
    });

    test(`board in flight, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.flight);
      await shoot(page, `board-in-flight-${vp.tag}`, vp.width, errors);
    });

    test(`board waiting, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.waiting);
      await shoot(page, `board-waiting-${vp.tag}`, vp.width, errors);
    });

    test(`thread, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.flight);
      await page.locator(`#board .card[data-id="${fx.flight.review_thread_id}"]`).click();
      await expect(page.locator("#collab-panel")).toBeVisible();
      await shoot(page, `thread-${vp.tag}`, vp.width, errors);
    });

    test(`connect an agent, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.flight);
      await page.locator("#connect-open").click();
      await expect(page.locator("#connect-dialog")).toBeVisible();
      await shoot(page, `connect-${vp.tag}`, vp.width, errors);
    });

    test(`empty channel, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.empty);
      await shoot(page, `empty-channel-${vp.tag}`, vp.width, errors);
    });
  });
}
