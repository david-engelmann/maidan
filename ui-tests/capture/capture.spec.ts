import { existsSync, readFileSync, mkdirSync } from "fs";
import { resolve } from "path";
import { test, expect, type Locator, type Page } from "@playwright/test";

// Captures the screens in the screenshot bar (docs/UI Design.md) from the real
// /ui, against the seed in crates/maidan-server/examples/capture_server.rs.
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

// Sign in the way the quickstart does: paste the workspace id and token, and
// the page trades them for a browser session.
async function signIn(page: Page, board: Board) {
  await page.clock.setFixedTime(new Date(fx.now));
  await page.goto("/ui/");
  await expect(page.locator("#session-status")).toBeHidden();
  await page.fill("#workspace", board.workspace_id);
  await page.fill("#token", board.token);
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator("#logout")).toBeVisible();
  // Come back to the console: the session made from the token opens the
  // board and Live, as it does for a person returning to the tab.
  await page.reload();
}

async function openBoard(page: Page, board: Board, cards: number) {
  await signIn(page, board);
  await expect(page.locator("#first-run")).toBeHidden();
  await expect(page.locator("#identity-who")).toHaveText(/David/);
  await expect(page.locator("#board-title")).toHaveText("# build");
  if (cards > 0) await expect(page.locator("#board .card")).toHaveCount(cards);
  await expect(page.locator("#board")).not.toHaveAttribute("aria-busy", "true");
}

// What every first screen must not show (UI Design, "Never on the first screen").
async function calmFirstScreen(page: Page) {
  await expect(page.locator("aside")).toBeHidden();
  await expect(page.locator("#tools")).not.toHaveAttribute("open", "");
  await expect(page.locator("#collab-panel")).toBeHidden();
  await expect(page.locator("#board-refusal")).toBeHidden();
  await expect(page.locator("#board .chrome-badge")).toHaveCount(0);
  await expect(page.locator(".legend-box")).toHaveCount(0);
  await expect(page.locator("#mint-banner")).toBeHidden();
}

async function filled(page: Page): Promise<Locator> {
  return page.locator("button.primary:visible");
}

async function shoot(
  page: Page,
  name: string | null,
  target?: Locator,
  animations: "disabled" | "allow" = "disabled",
): Promise<Buffer> {
  // Settle: the live status, web fonts, and two frames after the last paint.
  await expect(page.locator("#ws-status")).toHaveText("connected");
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  });
  await page.mouse.move(0, 0);
  // A null name returns the PNG without writing it.
  const opts = {
    path: name === null ? undefined : resolve(OUT, `${name}.png`),
    animations,
    caret: "hide" as const,
    scale: "css" as const,
  };
  return target ? target.screenshot(opts) : page.screenshot(opts);
}

test("board, work in flight", async ({ page }) => {
  await openBoard(page, fx.flight, 7);
  await calmFirstScreen(page);
  await expect(page.locator("#needs-you-quiet")).toHaveText("Nothing is waiting on you.");
  for (const lane of ["Open", "In progress", "Needs review", "Done"]) {
    await expect(page.locator("#board .board-col")).toContainText([lane]);
  }
  await expect(page.locator('#board .card-state:text-is("running")').first()).toBeVisible();
  await expect(page.locator("#board .card .card-foot .name", { hasText: "Server coder" }).first()).toBeVisible();
  await expect(await filled(page)).toHaveCount(0);
  await shoot(page, "board-in-flight");
});

test("board, something waiting", async ({ page }) => {
  await openBoard(page, fx.waiting, 7);
  await calmFirstScreen(page);
  await expect(page.locator("#needs-you-list .ny-item")).toHaveCount(1);
  await expect(await filled(page)).toHaveCount(1);
  await expect(await filled(page)).toHaveText("Approve");
  await expect(page.locator(".ny-evidence .ny-ev-tier").first()).toBeVisible();
  await shoot(page, "board-waiting");
});

test("refused close", async ({ page }) => {
  await openBoard(page, fx.waiting, 7);
  const card = page.locator(`#board .card[data-id="${fx.waiting.review_thread_id}"]`);
  await expect(card.locator(".card-refusal")).toHaveText("Close refused");
  await expect(page.locator("#board-refusal")).toBeHidden();
  await shoot(page, "refused-close", page.locator("#board-panel"));
});

test("thread", async ({ page }) => {
  // The task in review, opened from its card on the board where nothing waits
  // on David. On the waiting board its Approve would sit beside Needs you's
  // Approve, two filled buttons, and the bar allows one.
  await openBoard(page, fx.flight, 7);
  await page.locator(`#board .card[data-id="${fx.flight.review_thread_id}"]`).click();
  const panel = page.locator("#collab-panel");
  await expect(panel).toBeVisible();
  await expect(page.locator("#message-list")).toContainText("re-checks the token or session");
  await expect(page.locator("#message-list")).not.toContainText("{");
  await expect(await filled(page)).toHaveCount(1);
  // Every shot is the 1440 x 900 viewport. The panel's foot sits at the
  // bottom of it, so the card it was opened from shows above the thread.
  await panel.evaluate((el) => el.scrollIntoView({ block: "end" }));
  await shoot(page, "thread");
});

test("connect an agent", async ({ page }) => {
  await openBoard(page, fx.flight, 7);
  await page.locator("#connect-open").click();
  await expect(page.locator("#connect-dialog")).toBeVisible();
  await expect(await filled(page)).toHaveCount(1);
  await expect(await filled(page)).toHaveText("Create member and mint token");
  await shoot(page, "connect");
});

test("empty channel", async ({ page }) => {
  await openBoard(page, fx.empty, 0);
  await calmFirstScreen(page);
  await expect(page.locator("#board-onboard")).toBeVisible();
  await expect(page.locator("#board .board-col")).toHaveCount(0);
  await expect(await filled(page)).toHaveCount(1);
  await expect(await filled(page)).toHaveText("Connect an agent");
  await shoot(page, "empty-channel");
});

test("could not load", async ({ page }) => {
  // The server is real; only the board's own read is cut, the way an
  // unreachable server looks to the page.
  await page.route(`**/channels/${fx.flight.channel_id}/threads**`, (route) => route.abort("connectionrefused"));
  await signIn(page, fx.flight);
  const error = page.locator("#board .onboard.board-error");
  await expect(error).toBeVisible();
  await expect(error.locator("button.primary")).toHaveText("Try again");
  await expect(page.locator("#board")).not.toContainText("{");
  await expect(page.locator("#board")).not.toContainText("HTTP");
  await expect(await filled(page)).toHaveCount(1);
  await shoot(page, "error");
});

test("reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openBoard(page, fx.flight, 7);
  await calmFirstScreen(page);
  // The bar's reduced-motion screen is "the same board", so it is not a file
  // of its own: it is captured with animations allowed (under reduced motion
  // nothing moves, so nothing can be mid-glide) and must be byte-identical to
  // board-in-flight.png, which the first test wrote to the same place. Any
  // difference is motion the guard let through. Run the whole capture.
  const shot = await shoot(page, null, undefined, "allow");
  const flight = resolve(OUT, "board-in-flight.png");
  if (!existsSync(flight)) throw new Error(`${flight} is missing: run the whole capture, not one test`);
  expect(shot.equals(readFileSync(flight)), "reduced motion must match board-in-flight.png").toBe(true);
});
