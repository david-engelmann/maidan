import { mkdirSync, readFileSync, writeFileSync } from "fs";
import { basename, resolve } from "path";
import { test, expect, type Page } from "@playwright/test";

// The review gallery: the screens a person sees first, at a desktop, two
// tablet and a phone width, so a UI change is looked at before it merges
// rather than after it ships. The pictures are for eyes; three things are
// checked because no eye should have to: a screen that scrolls sideways on its
// own viewport, a page that throws, and a page that logs a console error (a
// refused request the screen should never have sent shows up there). Each
// fails the run, and report.json lists every screen with what was measured.

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
// The server answers `/` and its /landing/ assets only where sign-up is on,
// and the capture seed leaves it off, so both come from the files build.rs
// wrote. Served by the server instead, the assets 404ed and every landing
// shot was the unstyled page.
const LANDING_DIST = resolve(__dirname, "../../crates/maidan-server/static/landing/dist");
async function openLanding(page: Page) {
  await page.clock.setFixedTime(new Date(fx.now));
  await page.route((url) => url.pathname === "/", (route) =>
    route.fulfill({ path: resolve(LANDING_DIST, "index.html"), contentType: "text/html; charset=utf-8" }),
  );
  await page.route((url) => url.pathname.startsWith("/landing/"), (route) =>
    route.fulfill({ path: resolve(LANDING_DIST, basename(new URL(route.request().url()).pathname)) }),
  );
  await page.goto("/", { waitUntil: "networkidle" });
}
const OUT = resolve(__dirname, "..", process.env.GALLERY_OUT ?? "gallery-out");
mkdirSync(OUT, { recursive: true });

// The tablet widths are where Files once scrolled sideways (a select that
// escaped its card at 900px) while 1440 and 390 both looked fine.
const VIEWPORTS = [
  { tag: "desktop", width: 1440, height: 900, mobile: false },
  { tag: "tablet", width: 1024, height: 768, mobile: false },
  { tag: "tablet-portrait", width: 768, height: 1024, mobile: false },
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

// Sign in the way the quickstart does: paste the token, and the console
// trades it for a browser session. The board's channel is set first.
async function signIn(page: Page, board: Board, hash = "#/board") {
  await page.clock.setFixedTime(new Date(fx.now));
  await page.addInitScript((id) => window.localStorage.setItem("maidan_console_channel", id), board.channel_id);
  await page.goto(`/ui/${hash}`);
  await page.getByLabel(/paste a token/i).fill(board.token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeHidden();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
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
      page.on("console", (m) => {
        if (m.type() === "error") errors.push(`console: ${m.text()}${m.location().url ? ` (${new URL(m.location().url).pathname})` : ""}`);
      });
      // A stylesheet, script, font or image that fails leaves a page that
      // throws nothing and looks broken.
      page.on("response", (r) => {
        const kind = r.request().resourceType();
        if (r.status() >= 400 && ["stylesheet", "script", "font", "image"].includes(kind)) {
          errors.push(`${kind} ${r.status()}: ${new URL(r.url()).pathname}`);
        }
      });
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
      await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeVisible();
      await shoot(page, `console-signed-out-${vp.tag}`, vp.width, errors);
    });

    test(`board in flight, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.flight);
      // A phone shows one lane at a time; a desktop shows all seven tasks.
      if (vp.mobile) await expect(page.locator("button.task").first()).toBeVisible();
      else await expect(page.locator("button.task")).toHaveCount(7);
      await shoot(page, `board-in-flight-${vp.tag}`, vp.width, errors);
    });

    test(`needs you, waiting, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.waiting, "#/needs");
      await expect(page.locator(".needs-count")).toHaveText("1 waiting on you");
      // The reviewer reads the evidence file before Approve.
      await expect(page.getByText("ws_deactivated_member_e2e: 3 passed")).toBeVisible();
      await shoot(page, `needs-waiting-${vp.tag}`, vp.width, errors);
    });

    test(`thread, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.flight);
      await page.locator("button.task").first().click();
      await expect(page.getByRole("dialog")).toBeVisible();
      await shoot(page, `thread-${vp.tag}`, vp.width, errors);
    });

    // The capture seed has no identity provider, so discovery is answered as
    // a server with one would answer it: the card leads with Sign in.
    test(`console signed out, identity provider, ${vp.tag}`, async ({ page }) => {
      await page.clock.setFixedTime(new Date(fx.now));
      await page.route("**/.well-known/maidan.json", async (route) => {
        const res = await route.fetch();
        const body = (await res.json()) as { auth?: Record<string, unknown> };
        await route.fulfill({ response: res, json: { ...body, auth: { ...(body.auth ?? {}), oidc: true, oidc_login: "/auth/oidc/login", sessions: true } } });
      });
      await page.goto("/ui/");
      await expect(page.getByRole("link", { name: "Sign in", exact: true })).toBeVisible();
      await expect(page.getByLabel("Or paste a token")).toBeVisible();
      await shoot(page, `console-signed-out-idp-${vp.tag}`, vp.width, errors);
    });

    test(`task in review, ${vp.tag}`, async ({ page }) => {
      test.skip(!fx.waiting.review_thread_id, "the seed has no task in review");
      // The task's own view: its messages, and its result and evidence opened.
      await signIn(page, fx.waiting, `#/threads/t/${fx.waiting.review_thread_id}`);
      // The deep link now moves the list to the task's channel, so the list
      // row (hidden behind the open conversation on a phone) also has the title.
      await expect(page.locator(".conv-open").getByText("End a deactivated member's WebSocket streams").first()).toBeVisible();
      await expect(page.getByText(/re-checks the token or session/).first()).toBeVisible();
      const more = page.locator("details", { has: page.locator("summary", { hasText: "Result and evidence" }) });
      await more.locator("summary").click();
      await expect(more.getByText(/3 of 3 caught/)).toBeVisible();
      await expect(more.locator("li").first()).toBeVisible();
      await shoot(page, `task-in-review-${vp.tag}`, vp.width, errors);
    });

    test(`connect an agent, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.flight, "#/tools/connect");
      await expect(page.getByRole("heading", { name: "Connect an agent" })).toBeVisible();
      await shoot(page, `connect-${vp.tag}`, vp.width, errors);
    });

    // The places a first visit reaches from the rail, after their data loads.
    for (const place of ["people", "activity", "catalog", "artifacts"] as const) {
      test(`${place}, ${vp.tag}`, async ({ page }) => {
        await signIn(page, fx.flight, `#/${place}`);
        const main = page.locator("#main");
        await expect(main).not.toBeEmpty();
        await expect(main.locator('[aria-busy="true"], .ghost-line')).toHaveCount(0);
        await shoot(page, `${place}-${vp.tag}`, vp.width, errors);
      });
    }

    test(`empty channel, ${vp.tag}`, async ({ page }) => {
      await signIn(page, fx.empty);
      await expect(page.locator("button.task")).toHaveCount(0);
      await shoot(page, `empty-channel-${vp.tag}`, vp.width, errors);
    });
  });
}
