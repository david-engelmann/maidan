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
// locale, timezone and color scheme are fixed in capture.config.ts.

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
async function signIn(page: Page, board: Board, hash: string) {
  await page.clock.setFixedTime(new Date(fx.now));
  await page.addInitScript((id) => window.localStorage.setItem("maidan_console_channel", id), board.channel_id);
  await page.goto(`/ui/${hash}`);
  await page.getByLabel(/paste a token/i).fill(board.token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeHidden();
}

async function settle(page: Page) {
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  });
  await page.mouse.move(0, 0);
}

async function shoot(page: Page, name: string, target?: Locator) {
  await settle(page);
  const opts = { path: resolve(OUT, `${name}.png`), animations: "disabled" as const, caret: "hide" as const, scale: "css" as const };
  return target ? target.screenshot(opts) : page.screenshot(opts);
}

test("board, work in flight", async ({ page }) => {
  await signIn(page, fx.flight, "#/board");
  for (const lane of ["Open", "In progress", "Needs review", "Done"]) {
    await expect(page.locator("h2.lane-head", { hasText: lane })).toBeVisible();
  }
  await expect(page.locator("button.task")).toHaveCount(7);
  await shoot(page, "board-in-flight");
});

test("needs you, something waiting", async ({ page }) => {
  await signIn(page, fx.waiting, "#/needs");
  await expect(page.locator(".needs-count")).toHaveText("1 waiting on you");
  await shoot(page, "board-waiting");
});

test("empty channel", async ({ page }) => {
  await signIn(page, fx.empty, "#/board");
  await expect(page.locator(".empty-state").first()).toBeVisible();
  await expect(page.locator("button.task")).toHaveCount(0);
  await shoot(page, "empty-channel");
});
