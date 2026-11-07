import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
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

const MESSAGE = {
  name: "post_message",
  title: "Post a message",
  description: "Post a message to a thread.",
  inputSchema: {
    type: "object",
    required: ["thread_id"],
    anyOf: [{ required: ["body"] }, { required: ["content"] }],
    properties: {
      thread_id: { type: "string", format: "uuid", description: "The thread to post in." },
      body: { type: "string", description: "Plain text." },
      content: { type: "array", items: { type: "object" } },
      kind: { anyOf: [{ type: "string", enum: ["note", "result"] }, { type: "null" }] },
      tags: { type: "array", items: { type: "string", enum: ["alpha", "beta"] } },
      inclusion: { type: "string", enum: ["pointer", "quote"], default: "pointer" },
    },
  },
  annotations: { destructiveHint: false, idempotentHint: false },
};

async function assertNoHorizontalOverflow(page: Page) {
  const extra = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(extra).toBeLessThanOrEqual(0);
}

test("role and safety chips filter the list and a search can be cleared", async ({ page }) => {
  await stubCatalog(page, 200, { tools: TOOLS });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, "#/catalog");
  const rows = page.locator(".tool-row");
  await expect(rows).toHaveCount(3);
  await expect(page.locator(".roster-count")).toHaveText("3 tools");
  const filters = page.getByRole("group", { name: "Filter tools" });
  await expect(filters.getByRole("button")).toHaveText(["All", "Worker", "Reviewer", "Coordinator", "Read-only", "Destructive"]);

  await filters.getByRole("button", { name: "Worker", exact: true }).click();
  await expect(filters.getByRole("button", { name: "Worker", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("Post a message");
  await expect(page.locator(".roster-count")).toHaveText("1 of 3 tools · /mcp/worker");

  await filters.getByRole("button", { name: "Read-only", exact: true }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("List threads");
  await expect(page.locator(".roster-count")).toHaveText("1 of 3 tools");

  await filters.getByRole("button", { name: "Destructive", exact: true }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows).toContainText("Delete a thread");
  await expect(page.locator(".roster-count")).toHaveText("1 of 3 tools");

  await page.getByLabel("Find a tool by name or description").fill("missing");
  await expect(page.locator(".catalog-list .empty-state")).toContainText("No tool matches “missing”.");
  await page.getByRole("button", { name: "Show all tools" }).click();
  await expect(page.locator(".roster-count")).toHaveText("3 tools");
  await expect(rows).toHaveCount(3);
});

test("a conditional schema labels one-of fields, nullable enums and the copied name", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await stubCatalog(page, 200, { tools: [MESSAGE] });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, "#/catalog/post_message");
  const detail = page.locator(".tool-detail");
  await expect(detail).toContainText("Requires one of: body, content");
  await expect(detail.locator(".param").filter({ hasText: "body" }).locator(".param-opt")).toHaveText("one of required");
  await expect(detail.locator(".param").filter({ hasText: "content" }).locator(".param-opt")).toHaveText("one of required");
  await expect(detail.locator(".param").filter({ hasText: "thread_id" }).locator(".param-req")).toHaveText("required");
  await expect(detail.locator(".param").filter({ hasText: "thread_id" })).toContainText("uuid");
  await expect(detail.locator(".param").filter({ hasText: "kind" })).toContainText("enum?");
  await expect(detail.locator(".param").filter({ hasText: "kind" }).locator(".param-choices code")).toHaveText(["note", "result"]);
  await expect(detail.locator(".param").filter({ hasText: "tags" }).locator(".param-choices code")).toHaveText(["alpha", "beta"]);
  await expect(detail.locator(".param").filter({ hasText: "inclusion" })).toContainText("default: pointer");
  await expect(detail).toContainText("/mcp/worker");
  await expect(detail).toContainText("/mcp/reviewer");
  await expect(detail).toContainText("/mcp/coordinator");
  await page.getByRole("button", { name: "Copy tool name" }).click();
  await expect(page.getByRole("button", { name: "Copy tool name" })).toHaveText("Copied");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("post_message");
});

test.describe("catalog on a phone", () => {
  // Measured mid-way through the route's enter translate, a 44px chip on a
  // fractional row reads 43.999996px. Reduced motion lays the page out still,
  // and the half-pixel slack below absorbs the rest of the float noise.
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, reducedMotion: "reduce" });

  test("the catalog fits a phone, controls are 44px, and axe finds nothing serious", async ({ page }) => {
    await stubCatalog(page, 200, { tools: TOOLS });
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.channel_id, "#/catalog");
    await expect(page.getByRole("group", { name: "Filter tools" })).toBeVisible();
    await assertNoHorizontalOverflow(page);
    const small = await page.locator(".catalog-list button:visible, .catalog-list input:visible, .catalog-list a.tool-row:visible").evaluateAll((els) =>
      els
        .map((e) => {
          const r = e.getBoundingClientRect();
          const name = (e.getAttribute("aria-label") || e.textContent || "").trim().replace(/\s+/g, " ").slice(0, 40);
          return { name, w: r.width, h: r.height };
        })
        .filter(({ w, h }) => h < 43.5 || w < 43.5)
        .map(({ name, w, h }) => `${name} ${Math.round(w)}x${Math.round(h)}`),
    );
    expect(small).toEqual([]);
    const listed = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(listed.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);

    await page.locator(".tool-row", { hasText: "Post a message" }).click();
    await expect(page.getByRole("button", { name: "Copy tool name" })).toBeVisible();
    await assertNoHorizontalOverflow(page);
    const detailSmall = await page.locator(".tool-detail button:visible, .member-detail a.back:visible").evaluateAll((els) =>
      els
        .map((e) => {
          const r = e.getBoundingClientRect();
          return { name: (e.textContent || "").trim().slice(0, 40), w: r.width, h: r.height };
        })
        .filter(({ h }) => h < 43.5)
        .map(({ name, w, h }) => `${name} ${Math.round(w)}x${Math.round(h)}`),
    );
    expect(detailSmall).toEqual([]);
    const opened = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(opened.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);
  });
});

test("the workspace chooser traps Tab and closes from Escape or the backdrop", async ({ page }) => {
  await signInWithCookie(page, fx);
  await page.goto("/ui/?choose_workspace=1#/catalog");
  const chooser = page.getByRole("dialog", { name: "Choose a workspace" });
  await expect(chooser).toBeVisible();
  const controls = chooser.locator("button, a[href]");
  await expect(controls).toHaveCount(2);
  await expect(controls.first()).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(controls.last()).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(controls.first()).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(controls.last()).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(controls.first()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(chooser).toBeHidden();

  await page.goto("/ui/?choose_workspace=1#/catalog");
  await expect(chooser).toBeVisible();
  await page.locator(".chooser-scrim").click({ position: { x: 8, y: 8 } });
  await expect(chooser).toBeHidden();
});

test("a failed workspace list does not leave the chooser open", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (err) => errors.push(err.message));
  await page.route(/\/auth\/session\/workspaces/, (route) => route.fulfill({ status: 500, contentType: "application/json", body: "{}" }));
  await signInWithCookie(page, fx);
  const asked = page.waitForResponse((r) => r.url().includes("/auth/session/workspaces"));
  await page.goto("/ui/?choose_workspace=1#/catalog");
  await asked;
  await expect(page.getByRole("dialog", { name: "Choose a workspace" })).toHaveCount(0);
  expect(errors).toEqual([]);
});
