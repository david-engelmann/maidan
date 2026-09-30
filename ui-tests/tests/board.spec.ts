import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

async function openBoard(page: Page) {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click("#refresh-channels");
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
}

// The channel renders as a board. Every thread sits in the lane its real FSM
// state and claim put it in — nothing is a catch-all "idle" — and a held task
// names its holder by display name, not by member id.
test("the board puts each thread in its real lane with its holder's name", async ({ page }) => {
  await openBoard(page);

  await expect(page.locator("#board-title")).toHaveText("# build");
  const card = (id: string) => page.locator(`#board .card[data-id="${id}"]`);
  const lane = (key: string) => page.locator(`#board .board-col[data-column="${key}"]`);

  await expect(lane("open").locator(`.card[data-id="${fx.board_open_thread_id}"]`)).toBeVisible();
  await expect(card(fx.board_open_thread_id)).toHaveAttribute("data-chrome", "open");
  await expect(card(fx.board_open_thread_id)).toContainText("unclaimed");

  await expect(lane("working").locator(`.card[data-id="${fx.board_claimed_thread_id}"]`)).toBeVisible();
  await expect(card(fx.board_claimed_thread_id)).toHaveAttribute("data-chrome", "claimed");
  await expect(card(fx.board_claimed_thread_id).locator(".person .name")).toHaveText("Deployer");
  await expect(card(fx.board_claimed_thread_id).locator(".person")).toHaveAttribute("title", fx.requester_id);

  await expect(lane("review").locator(`.card[data-id="${fx.board_review_thread_id}"]`)).toBeVisible();
  await expect(card(fx.board_review_thread_id)).toHaveAttribute("data-chrome", "in review");
  await expect(lane("done").locator(`.card[data-id="${fx.board_done_thread_id}"]`)).toBeVisible();

  // The summary counts what the lanes show.
  await expect(page.locator("#board-summary")).toContainText("4 tasks");
  await expect(page.locator("#board-summary")).toContainText("1 agent working");
  await expect(page.locator("#board-summary")).toContainText("1 waiting on review");

  // Each thread is drawn once, on the board, with its real state; the
  // sidebar lists channels only.
  await expect(page.locator("#thread-list")).toHaveCount(0);
  const badges = page.locator("#board .card .chrome-badge");
  await expect(badges).toHaveCount(4);
  await expect(page.locator("#board")).not.toContainText("idle");
  await expect(
    page.locator(`#board .card[data-id="${fx.board_claimed_thread_id}"] .card-foot .name`),
  ).toHaveText("Deployer");
});

// Opening a card shows the thread with its state, holder and result, and every
// message is attributed by name with an agent/human tag.
test("a thread shows its state, holder, result and authors by name", async ({ page }) => {
  await openBoard(page);
  await page.click(`#board .card[data-id="${fx.board_review_thread_id}"]`);

  await expect(page.locator("#thread-context")).toHaveText("Review: result waiting on a reviewer");
  await expect(page.locator("#thread-badge .chrome-badge")).toHaveText("in review");
  await expect(page.locator("#thread-facts")).toContainText("held by");
  await expect(page.locator("#thread-facts")).toContainText("Deployer");
  // The result reads as a field, not as JSON.
  await expect(page.locator("#thread-facts .result .kv .k")).toHaveText("status");
  await expect(page.locator("#thread-facts .result .kv .v")).toHaveText("fixed");
  await expect(page.locator("#thread-facts")).not.toContainText("{");

  const msg = page.locator("#message-list .msg").first();
  await expect(msg.locator(".meta .person .name")).toHaveText("Deployer");
  await expect(msg.locator(".meta .kind-tag")).toHaveText("agent");
  await expect(msg.locator(".body")).toHaveText("Done; result attached.");
  // No raw member id is printed as text anywhere in the thread.
  await expect(page.locator("#message-list")).not.toContainText(fx.requester_id);
});

// The Live bar is one slim row until the socket connects; the raw event feed
// is opt-in even then.
test("the Live bar stays collapsed until connected, and the raw feed is opt-in", async ({ page }) => {
  await openBoard(page);
  // A pasted token signs in and folds the inputs into the identity pill.
  await page.click("#conn-edit");
  await page.fill("#token", fx.live_token);
  const feed = page.locator("#live-feed");
  await expect(feed).toBeHidden();
  await expect(page.locator("#live-toggle")).toBeHidden();
  const bar = await page.locator("#live-panel").boundingBox();
  expect(bar!.height).toBeLessThan(80);

  await page.click("#ws-connect");
  await expect(page.locator("#ws-status")).toHaveText("connected");
  await expect(page.locator("#live-panel")).toHaveClass(/connected/);
  await expect(feed).toBeHidden();
  // Raw events live in the Live menu, not on the first screen.
  await page.locator("#live-more summary").click();
  await page.click("#live-toggle");
  await expect(feed).toBeVisible();
  await expect(feed).toContainText("[ack]");
});

// A browser where an older page stored the token opens straight onto its last
// board: the token is exchanged for a session once and removed, the
// connection inputs collapse into an identity pill, and Live connects on its
// own.
test("a remembered token opens the last board, signed in and live", async ({ page }) => {
  await page.addInitScript(
    ([ws, tok, ch]) => {
      localStorage.setItem("maidan_workspace", ws);
      localStorage.setItem("maidan_token", tok);
      localStorage.setItem("maidan_channel", ch);
    },
    [fx.workspace_id, fx.live_token, fx.board_channel_id],
  );
  await page.goto("/ui/");
  await expect(page.locator("#identity-pill")).toBeVisible();
  await expect(page.locator("#identity-who")).toContainText("Operator");
  await expect(page.locator("#conn-fields")).toBeHidden();
  await expect(page.locator("#board-title")).toHaveText("# build");
  await expect(page.locator("#ws-status")).toHaveText("connected");

  await page.click("#conn-edit");
  await expect(page.locator("#conn-fields")).toBeVisible();
  await expect(page.locator("#token")).toHaveValue("");
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBeNull();
});

// A token the server refuses to subscribe says why, instead of hanging on
// "connecting…".
test("a refused subscribe shows the server's reason", async ({ page }) => {
  await openBoard(page);
  await page.click("#ws-connect");
  await expect(page.locator("#ws-status")).toContainText("missing event:subscribe capability");
  await expect(page.locator("#ws-status")).toHaveClass(/error/);
});
