import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

async function signIn(page: Page) {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.locator("#token").dispatchEvent("change");
  await page.click("#refresh-channels");
  await expect(page.locator(`#channel-list li[data-id="${fx.board_channel_id}"]`)).toBeVisible();
}

// Ctrl/Cmd+K opens the palette; words in any order find a channel, and Enter
// jumps there.
test("the palette jumps to a channel and to a task by keyboard", async ({ page }) => {
  await signIn(page);
  // The shortcut is labelled the way this keyboard names it.
  const shortcut = await page.evaluate(() =>
    /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent) ? "⌘K" : "Ctrl K",
  );
  await expect(page.locator("#palette-open .kbd")).toHaveText(shortcut);
  await page.locator("body").click();
  await page.keyboard.press("ControlOrMeta+k");
  const palette = page.locator("#palette");
  await expect(palette).toBeVisible();
  await expect(page.locator("#palette-input")).toBeFocused();

  await page.keyboard.type("channel build");
  await expect(page.locator("#palette-list li[aria-selected='true'] .pl")).toHaveText("# build");
  await page.keyboard.press("Enter");
  await expect(palette).toBeHidden();
  await expect(page.locator("#board-title")).toHaveText("# build");

  // "/" opens it too; arrows move, Enter runs the selected task.
  await page.locator("#board-title").click();
  await page.keyboard.press("/");
  await expect(palette).toBeVisible();
  await page.keyboard.type("task review result");
  await expect(page.locator("#palette-list li")).toHaveCount(1);
  await expect(page.locator("#palette-list li .kbd")).toHaveText("in review");
  await page.keyboard.press("Enter");
  await expect(page.locator("#thread-context")).toHaveText("Review: result waiting on a reviewer");

  // Nothing matching says what to try; Escape closes.
  await page.keyboard.press("ControlOrMeta+k");
  await page.keyboard.type("zzzz nothing");
  await expect(page.locator("#palette-list li.empty")).toContainText("Nothing matches");
  await page.keyboard.press("Escape");
  await expect(palette).toBeHidden();
});

// Before a channel is picked the board explains both sides; an empty channel
// says how tasks arrive. Both lead to Connect an agent.
test("the empty board and an empty channel explain how work arrives", async ({ page }) => {
  await page.goto("/ui/");
  const onboard = page.locator("#board-onboard");
  await expect(onboard).toContainText("A task arrives when an agent or a person opens one.");
  await expect(onboard.getByRole("button", { name: "Connect an agent" })).toBeVisible();
  await expect(onboard).not.toContainText("claim_next_thread");
  await expect(onboard).not.toContainText("pick a channel");
  await expect(page.locator("#board .board-col")).toHaveCount(0);

  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.locator("#token").dispatchEvent("change");
  await page.click("#refresh-channels");
  await page.click(`#channel-list li[data-id="${fx.quiet_channel_id}"]`);
  await expect(onboard).toContainText("A task arrives when an agent or a person opens one.");
  await expect(onboard).not.toContainText("No tasks in #quiet");
  await expect(page.locator("#board .board-col")).toHaveCount(0);
  await onboard.getByRole("button", { name: "Connect an agent" }).click();
  await expect(page.locator("#connect-dialog")).toBeVisible();
});

// Connect an agent hands over the MCP endpoint of this server in the shapes
// clients take, with a placeholder token: never the viewer's own.
test("connect an agent gives copyable MCP config for this server", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await signIn(page);
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await page.click("#connect-open");
  const dialog = page.locator("#connect-dialog");
  await expect(dialog).toBeVisible();

  const url = `${fx.base_url}/mcp/streamable`;
  const json = JSON.parse((await page.locator("#cx-json").textContent()) || "{}");
  expect(json.mcpServers.maidan.url).toBe(url);
  expect(json.mcpServers.maidan.headers.Authorization).toBe("Bearer REPLACE_WITH_MAIDAN_TOKEN");
  await expect(page.locator("#cx-claude")).toContainText(`claude mcp add --transport http maidan ${url}`);
  await expect(page.locator("#cx-prompt")).toContainText(`claim_next_thread with channel_id ${fx.board_channel_id} (#build) and lease_secs 900`);
  await expect(page.locator("#cx-prompt")).toContainText(`${fx.base_url}/llms.txt`);
  await expect(dialog).not.toContainText(fx.token);

  const href = (await page.locator("#cx-cursor").getAttribute("href")) || "";
  expect(href.startsWith("cursor://anysphere.cursor-deeplink/mcp/install?name=maidan&config=")).toBeTruthy();
  const config = JSON.parse(Buffer.from(decodeURIComponent(href.split("config=")[1]), "base64").toString());
  expect(config.url).toBe(url);

  const llms = await page.request.get((await page.locator("#cx-llms").getAttribute("href")) || "");
  expect(llms.ok()).toBeTruthy();
  expect(await llms.text()).toContain("# Maidan");

  await dialog.locator("[data-copy='cx-json']").click();
  await expect(page.locator("#cx-status")).toHaveText("Copied.");
  const clip = await page.evaluate(() => navigator.clipboard.readText());
  expect(JSON.parse(clip).mcpServers.maidan.url).toBe(url);

  await page.click("#cx-mint");
  await expect(dialog).toBeHidden();
  await expect(page.locator("#tools [data-tab='tokens']")).toHaveAttribute("aria-selected", "true");
});

// "Open next review" opens the oldest review waiting on me, even from another
// channel, and puts focus on its Approve button.
// Leaving the token field signs in, and sign-in ends by loading what is
// waiting on me. Clicking Refresh as well started a second channel load, and
// the board load after the row click starts another needs-you load. The test
// waited on none of them, so on a busy full run a needs-you answer could land
// after Enter. Rebuilding the list detaches the focused row, which drops its
// focus, and the last check saw no focused button. Same double load #1338
// fixed in board.spec: sign in by leaving the field, wait for that sign-in's
// channels and needs-you answers, then for the board's own needs-you answer,
// and only then open the palette.
test("the palette opens the next review waiting on me", async ({ page }) => {
  const get = (match: (url: string) => boolean) =>
    page.waitForResponse((r) => r.request().method() === "GET" && match(r.url()) && r.ok());
  const waiting = (url: string) => /\/members\/[^/]+\/waiting(\?|$)/.test(url);
  // No session yet, so this read answers 401. Any answer will do.
  const session = page.waitForResponse(
    (r) => r.request().method() === "GET" && r.url().endsWith("/auth/session"),
  );
  await page.goto("/ui/");
  await session;
  await page.fill("#workspace", fx.workspace_id);
  const channels = get((url) => url.includes(`/workspaces/${fx.workspace_id}/channels`));
  const signedIn = get(waiting);
  await page.fill("#token", fx.review_token);
  await page.locator("#token").blur();
  await channels;
  await (await signedIn).finished();

  const board = get((url) => url.includes(`/channels/${fx.board_channel_id}/threads`));
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await board;
  // The board asks for the needs-you queue only after its threads and gates.
  await (await get(waiting)).finished();
  // One more page task, so the handler for that answer has rendered.
  await page.evaluate(() => new Promise((r) => setTimeout(r, 0)));
  await expect(page.locator("#needs-you-list .ny-item").first()).toBeVisible();

  await page.locator("#board-title").click();
  await page.keyboard.press("ControlOrMeta+k");
  await page.keyboard.type("next review");
  const label = page.locator("#palette-list li[aria-selected='true'] .pl");
  await expect(label).toContainText("Open next review: ");
  await expect(label).not.toContainText("untitled");
  const title = ((await label.textContent()) || "").replace("Open next review: ", "");
  await page.keyboard.press("Enter");
  await expect(page.locator("#thread-context")).toHaveText(title);
  await expect(page.locator("#needs-you-list .ny-item button.primary:focus")).toHaveCount(1);
});

// Opening a task that is not on the board looks its channel up first. A
// channel picked before that answer arrives wins: the late answer must not
// drag the board back to the task's channel.
test("a late channel lookup for an opened task does not undo a channel picked since", async ({ page }) => {
  let release = () => {};
  const held = new Promise<void>((r) => (release = r));
  const lookup = new RegExp(`/threads/${fx.thread_id}$`);
  await page.route(lookup, async (route) => {
    await held;
    await route.continue();
  });
  await signIn(page);
  // Any row on the #general thread will do: other specs add gates to it.
  const row = page.locator(`#needs-you-list .ny-item[data-thread-id="${fx.thread_id}"] .ny-title`).first();
  const label = ((await row.textContent()) || "").trim();
  await row.click();
  await expect(page.locator("#thread-context")).toHaveText(label);
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await expect(page.locator("#board-title")).toHaveText("# build");

  const answered = page.waitForResponse(lookup);
  release();
  await (await answered).finished();
  // One more page task, so the handler for that answer has run before the check.
  await page.evaluate(() => new Promise((r) => setTimeout(r, 0)));
  await expect(page.locator("#board-title")).toHaveText("# build");
  await expect(page.locator("#channel-list li.selected")).toHaveAttribute("data-id", fx.board_channel_id);
});
