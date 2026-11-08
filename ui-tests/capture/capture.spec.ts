import { readFileSync, mkdirSync } from "fs";
import { resolve } from "path";
import { test, expect, type Locator, type Page } from "@playwright/test";

// Captures the screens in the screenshot bar (docs/UI Design.md) from the real
// console at /ui, against the seed in crates/maidan-server/examples/capture_server.rs.
// Each shot first asserts what the bar says a person must see. A screen that
// does not render that way fails the run, and its PNG is not written.
//
// Every run draws the same pixels: the seed pins member ids and timestamps,
// the browser clock is fixed to the seed's clock, and the viewport, scale,
// locale and timezone are fixed in capture.config.ts. Each screen is shot in
// both themes, set the way a person sets it (the console's stored choice):
// light under the names the README links, dark with a `-dark` suffix.

interface Board {
  workspace_id: string;
  channel_id: string;
  token: string;
  david_id: string;
  review_thread_id: string | null;
}
interface Capture {
  base_url: string;
  now: string;
  flight: Board;
  waiting: Board;
  empty: Board;
}

const fx: Capture = JSON.parse(readFileSync(resolve(__dirname, "../.capture.json"), "utf8"));
const OUT = resolve(__dirname, "..", process.env.CAPTURE_OUT ?? "../docs/assets/screens");
mkdirSync(OUT, { recursive: true });

// Sign in the way the quickstart does: paste the token, and the console
// trades it for a browser session. The board's channel is set first.
async function signIn(page: Page, board: Board, hash: string, theme: Theme) {
  await page.clock.setFixedTime(new Date(fx.now));
  await page.addInitScript(
    ([id, t]) => {
      window.localStorage.setItem("maidan_console_channel", id);
      window.localStorage.setItem("maidan_console_theme", t);
    },
    [board.channel_id, theme],
  );
  await page.goto(`/ui/${hash}`);
  await page.getByLabel(/paste a token/i).fill(board.token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeHidden();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
}

type Theme = "light" | "dark";
const THEMES: readonly Theme[] = ["light", "dark"];
const named = (name: string, theme: Theme) => (theme === "light" ? name : `${name}-dark`);

async function settle(page: Page) {
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  });
  await page.mouse.move(0, 0);
}

async function shoot(page: Page, name: string, theme: Theme, target?: Locator) {
  await settle(page);
  const opts = { path: resolve(OUT, `${named(name, theme)}.png`), animations: "disabled" as const, caret: "hide" as const, scale: "css" as const };
  return target ? target.screenshot(opts) : page.screenshot(opts);
}

for (const theme of THEMES) {
  test(`${theme}: board, work in flight`, async ({ page }) => {
    await signIn(page, fx.flight, "#/board", theme);
    for (const lane of ["Open", "In progress", "Needs review", "Done"]) {
      await expect(page.locator("h2.lane-head", { hasText: lane })).toBeVisible();
    }
    await expect(page.locator("button.task")).toHaveCount(7);
    await shoot(page, "board-in-flight", theme);
    // The README's hero: the lanes without the empty canvas below them.
    await page.screenshot({
      path: resolve(OUT, `${named("board-readme", theme)}.png`),
      animations: "disabled",
      caret: "hide",
      scale: "css",
      clip: { x: 0, y: 0, width: 1440, height: 424 },
    });
  });

  test(`${theme}: needs you, something waiting`, async ({ page }) => {
    await signIn(page, fx.waiting, "#/needs", theme);
    await expect(page.locator(".needs-count")).toHaveText("1 waiting on you");
    await shoot(page, "board-waiting", theme);
  });

  test(`${theme}: empty channel`, async ({ page }) => {
    await signIn(page, fx.empty, "#/board", theme);
    await expect(page.locator(".empty-state").first()).toBeVisible();
    await expect(page.locator("button.task")).toHaveCount(0);
    await shoot(page, "empty-channel", theme);
  });
}
