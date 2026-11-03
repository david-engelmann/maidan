import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Files: every file linked to a task, as a grid or a list, with a preview,
// its metadata and a link to the task it came from.
const fx = fixtures();
const auth = { Authorization: `Bearer ${fx.token}` };

async function linkedText(page: Page, filename: string, body: string): Promise<string> {
  const params = new URLSearchParams({ kind: "attachment", mime_type: "text/plain", filename });
  const up = await page.request.post(`/artifacts?${params}`, { headers: auth, data: body });
  expect(up.status()).toBe(201);
  const sha = (await up.json()).sha256 as string;
  const put = await page.request.put(`/threads/${fx.thread_id}/artifacts/${sha}`, { headers: auth });
  expect(put.ok()).toBeTruthy();
  return sha;
}

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
});

test("a linked file shows in the grid and list, opens with a preview and its task", async ({ page }) => {
  const name = `notes-${Date.now()}.txt`;
  await linkedText(page, name, "deploy went fine");
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/artifacts");
  const tile = page.locator(".file-tile", { hasText: name });
  await expect(tile).toBeVisible();
  await page.getByRole("button", { name: "List" }).click();
  await expect(page.locator(".files-list .file-tile", { hasText: name })).toBeVisible();
  await tile.getByRole("button").click();
  const detail = page.getByRole("region", { name: `File ${name}` });
  await expect(detail.locator(".file-pre")).toHaveText("deploy went fine");
  await expect(detail.locator("dd", { hasText: "text/plain" })).toBeVisible();
  await expect(detail.locator(".file-sources a")).toHaveAttribute("href", `#/threads/t/${fx.thread_id}`);
  await page.getByLabel("Search files").fill("zzz-nothing");
  await expect(page.getByText("No files match")).toBeVisible();
});

test("no files says how they arrive, and a failed read offers a retry", async ({ page }) => {
  await signInWithCookie(page, fx);
  await page.route(/\/threads\/[^/]+\/artifacts$/, (r) => r.fulfill({ status: 200, contentType: "application/json", body: "[]" }));
  await openOnChannel(page, fx.board_channel_id, "#/artifacts");
  await expect(page.getByText("No files yet", { exact: true })).toBeVisible();
  await page.unroute(/\/threads\/[^/]+\/artifacts$/);
  await page.route(/\/workspaces\/[^/]+\/channels$/, (r) => r.fulfill({ status: 500, contentType: "application/json", body: "{}" }));
  await page.reload();
  await expect(page.getByRole("alert").filter({ hasText: "Could not load the files." })).toBeVisible();
  await page.unroute(/\/workspaces\/[^/]+\/channels$/);
  await page.getByRole("button", { name: "Try again" }).click();
  await expect(page.locator(".file-tile").first()).toBeVisible();
});
