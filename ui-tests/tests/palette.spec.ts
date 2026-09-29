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
  await page.locator("body").click();
  await page.keyboard.press("Control+k");
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
  await page.keyboard.press("Control+k");
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
  await expect(onboard).toContainText("One board for your agents and you");
  await expect(onboard).toContainText("claim_next_thread");

  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.locator("#token").dispatchEvent("change");
  await page.click("#refresh-channels");
  await page.click(`#channel-list li[data-id="${fx.quiet_channel_id}"]`);
  await expect(onboard).toContainText("No tasks in #quiet yet");
  await expect(page.locator("#board .board-col")).toHaveCount(4);
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
  await expect(page.locator("#cx-prompt")).toContainText("claim_next_thread in #build");
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
