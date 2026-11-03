import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// The console's Catalog lists the MCP tools the caller can call, from
// GET /tools. The catalog answer is stubbed here so each state is shown on
// purpose: a full list with a filter and a detail, an empty list, and a 403.
const fx = fixtures();

const TOOLS = [
  {
    name: "post_message",
    title: "Post a message",
    description: "Post a message to a thread. It is delivered to everyone following it.",
    inputSchema: { type: "object", required: ["thread_id", "body"], properties: { thread_id: { type: "string", description: "The thread to post in." }, body: { type: "string" }, kind: { type: "string", enum: ["note", "result"] } } },
    annotations: { destructiveHint: false, idempotentHint: false },
  },
  {
    name: "list_threads",
    title: "List threads",
    description: "Threads in a channel, newest first.",
    inputSchema: { type: "object", properties: { channel_id: { type: "string" }, limit: { type: "integer", description: "At most this many." } } },
    annotations: { readOnlyHint: true, idempotentHint: true },
  },
  { name: "delete_thread", title: "Delete a thread", description: "Remove a thread.", inputSchema: { type: "object", properties: {} }, annotations: { destructiveHint: true } },
];

async function stubCatalog(page: Page, status: number, body: unknown) {
  await page.route(/\/(ui\/api\/)?tools(\?.*)?$/, (route) => route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) }));
}

test("the catalog lists tools, filters them, and opens one to its parameters", async ({ page }) => {
  await stubCatalog(page, 200, { tools: TOOLS });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, "#/catalog");
  const rows = page.locator(".tool-row");
  await expect(rows).toHaveCount(3);
  await expect(page.locator(".roster-count")).toHaveText("3 tools");
  await page.getByLabel("Find a tool by name or description").fill("thread newest");
  await expect(rows).toHaveCount(1);
  await expect(rows.first()).toContainText("List threads");
  await expect(rows.first().locator(".hint-badge.read-only")).toBeVisible();
  await page.getByLabel("Find a tool by name or description").fill("");
  await rows.filter({ hasText: "Post a message" }).click();
  await expect(page).toHaveURL(/#\/catalog\/post_message$/);
  const detail = page.locator(".tool-detail");
  await expect(detail.locator("h2")).toHaveText("Post a message");
  await expect(detail.locator(".param dt code")).toHaveText(["body", "thread_id", "kind"]);
  await expect(detail.locator(".param").nth(1)).toContainText("The thread to post in.");
  await expect(detail.locator(".param-req")).toHaveCount(2);
  await expect(detail.locator(".param-choices code")).toHaveText(["note", "result"]);
  await page.goto("/ui/#/catalog/delete_thread");
  await expect(page.locator(".tool-detail .hint-badge.destructive")).toHaveText("Destructive");
  await expect(page.locator(".tool-detail")).toContainText("This tool takes no parameters.");
});

test("an empty catalog says so", async ({ page }) => {
  await stubCatalog(page, 200, { tools: [] });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, "#/catalog");
  await expect(page.locator(".catalog-list .empty-state")).toContainText("This access lists no tools.");
});

test("a 403 names the missing capability", async ({ page }) => {
  await stubCatalog(page, 403, { error: "forbidden" });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, "#/catalog");
  await expect(page.getByRole("alert").filter({ hasText: "workspace:read" })).toBeVisible();
});
