import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

// The add itself is answered here, so the seeded workspace keeps its members
// for every other spec; first_hour_agents_e2e drives the real route.
const SECRET = "mdn_playwright_add_agent_secret";

test("a signed-in person adds an agent and gets its client setup with the token, once", async ({ page }) => {
  const sent: Array<{ url: string; body: unknown }> = [];
  await page.route("**/ui/api/workspaces/*/agents", async (route) => {
    sent.push({ url: route.request().url(), body: route.request().postDataJSON() });
    await route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({
        member: { id: "00000000-0000-7000-8000-000000000001", workspace_id: fx.workspace_id, handle: "playwright-bot", display_name: "Playwright Bot", kind: "agent" },
        token: { id: "00000000-0000-7000-8000-000000000002", secret: SECRET, workspace_id: fx.workspace_id, member_id: "00000000-0000-7000-8000-000000000001", capabilities: ["workspace:read"], quotas: [] },
      }),
    });
  });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/people");

  const panel = page.locator(".add-agent-card");
  await panel.getByLabel("Agent name").fill("Playwright Bot");
  await expect(panel.getByText("It joins as @playwright-bot")).toBeVisible();
  await panel.getByRole("button", { name: "Cursor" }).click();
  await panel.getByRole("button", { name: "Add agent" }).click();

  await expect(panel.getByText("Token for @playwright-bot")).toBeVisible();
  expect(sent).toHaveLength(1);
  expect(sent[0].url).toContain(`/ui/api/workspaces/${fx.workspace_id}/agents`);
  expect(sent[0].body).toEqual({ handle: "playwright-bot", display_name: "Playwright Bot" });

  const origin = new URL(fx.base_url).origin;
  const cursor = panel.getByRole("link", { name: "Add to Cursor" });
  await expect(cursor).toHaveAttribute("href", /^cursor:\/\/anysphere\.cursor-deeplink\/mcp\/install\?name=maidan&config=/);
  const href = (await cursor.getAttribute("href")) ?? "";
  const config = JSON.parse(Buffer.from(decodeURIComponent(new URL(href).searchParams.get("config") ?? ""), "base64").toString());
  expect(config).toEqual({ url: `${origin}/mcp/streamable`, headers: { Authorization: `Bearer ${SECRET}` } });

  await panel.getByRole("button", { name: "VS Code" }).click();
  await expect(panel.getByRole("link", { name: "Add to VS Code" })).toHaveAttribute("href", /^vscode:mcp\/install\?/);
  await panel.getByRole("button", { name: "Claude Code" }).click();
  await expect(panel.locator(".snippet-body")).toHaveText(`claude mcp add --transport http maidan ${origin}/mcp/streamable --header "Authorization: Bearer ${SECRET}"`);

  // "I have it" drops the token from the page.
  await panel.getByRole("button", { name: "I have it" }).click();
  await expect(page.getByText(SECRET)).toHaveCount(0);
  await expect(panel.getByLabel("Agent name")).toBeVisible();
});
