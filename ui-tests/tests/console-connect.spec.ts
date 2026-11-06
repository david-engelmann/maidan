import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

// The add itself is answered here, so the seeded workspace keeps its members
// for every other spec; first_hour_agents_e2e drives the real route.
const SECRET = "mdn_playwright_connect_mint_secret";

function cursorConfig(href: string): { url?: string; headers?: { Authorization?: string } } {
  const config = new URL(href).searchParams.get("config") ?? "";
  return JSON.parse(Buffer.from(decodeURIComponent(config), "base64").toString());
}

function vscodeConfig(href: string): { name?: string; type?: string; url?: string; headers?: { Authorization?: string } } {
  return JSON.parse(decodeURIComponent(href.slice(href.indexOf("?") + 1)));
}

async function placeholderHrefs(page: Page): Promise<string[]> {
  const hrefs = await page.locator("a").evaluateAll((els) => els.map((e) => e.getAttribute("href") ?? ""));
  return hrefs.filter((href) => href.includes("REPLACE_WITH_MAIDAN_TOKEN"));
}

async function sideways(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
}

// The shared client-setup row sizes its buttons at 36px. This page does not
// restyle that row; every other control on the Connect cards is checked.
async function shortTargets(page: Page): Promise<string[]> {
  return page.locator("section.card").locator("button:visible, a.button:visible, input:visible, summary:visible").evaluateAll((els) =>
    els
      .filter((el) => !el.closest(".add-agent .snippet-head"))
      .map((el) => {
        const r = el.getBoundingClientRect();
        const t = (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 40);
        return { t, w: r.width, h: r.height };
      })
      .filter(({ w, h }) => h < 44 || w < 44)
      .map(({ t, w, h }) => `${t} ${Math.round(w)}x${Math.round(h)}`),
  );
}

test.describe("connect on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("OAuth links carry only the MCP URL, and naming an agent mints its token once", async ({ page }) => {
    const sent: Array<{ url: string; body: unknown }> = [];
    await page.route("**/ui/api/workspaces/*/agents", async (route) => {
      sent.push({ url: route.request().url(), body: route.request().postDataJSON() });
      await route.fulfill({
        status: 201,
        contentType: "application/json",
        body: JSON.stringify({
          member: { id: "00000000-0000-7000-8000-0000000000c1", workspace_id: fx.workspace_id, handle: "connect-bot", display_name: "Connect Bot", kind: "agent" },
          token: { id: "00000000-0000-7000-8000-0000000000c2", secret: SECRET, workspace_id: fx.workspace_id, member_id: "00000000-0000-7000-8000-0000000000c1", capabilities: ["workspace:read"], quotas: [] },
        }),
      });
    });
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id, "#/tools/connect");

    const origin = new URL(fx.base_url).origin;
    const oauth = page.locator("section.card", { has: page.getByRole("heading", { name: "Connect an agent" }) });
    const tokenCard = page.locator("section.card", { has: page.getByRole("heading", { name: "Connect with a token" }) });
    const cursor = oauth.getByRole("link", { name: "Add to Cursor" });
    const vscode = oauth.getByRole("link", { name: "Add to VS Code" });
    await expect(cursor).toHaveAttribute("href", /^cursor:\/\/anysphere\.cursor-deeplink\/mcp\/install\?name=maidan&config=/);
    await expect(vscode).toHaveAttribute("href", /^vscode:mcp\/install\?/);
    const cursorHref = (await cursor.getAttribute("href")) ?? "";
    const vscodeHref = (await vscode.getAttribute("href")) ?? "";
    expect(cursorConfig(cursorHref)).toEqual({ url: `${origin}/mcp/streamable` });
    expect(vscodeConfig(vscodeHref)).toEqual({ name: "maidan", type: "http", url: `${origin}/mcp/streamable` });
    expect(cursorHref.includes("headers")).toBe(false);
    expect(vscodeHref.includes("headers")).toBe(false);
    expect(await placeholderHrefs(page)).toEqual([]);
    await expect(tokenCard.getByRole("link", { name: "Add to Cursor" })).toHaveCount(0);
    expect(await sideways(page)).toBeLessThanOrEqual(0);

    await tokenCard.getByLabel("Agent name").fill("Connect Bot");
    await tokenCard.getByRole("button", { name: "Add agent" }).click();
    await expect(tokenCard.getByText("Token for @connect-bot")).toBeVisible();
    await expect(tokenCard.locator(".secret-value")).toHaveAttribute("aria-label", "The secret, hidden");
    await expect(tokenCard.locator(".secret-value")).not.toContainText(SECRET);
    expect(sent).toHaveLength(1);
    expect(sent[0].url).toContain(`/ui/api/workspaces/${fx.workspace_id}/agents`);
    expect(sent[0].body).toEqual({ handle: "connect-bot", display_name: "Connect Bot" });

    await tokenCard.getByRole("button", { name: "Cursor" }).click();
    const mintedCursor = (await tokenCard.getByRole("link", { name: "Add to Cursor" }).getAttribute("href")) ?? "";
    expect(cursorConfig(mintedCursor)).toEqual({ url: `${origin}/mcp/streamable`, headers: { Authorization: `Bearer ${SECRET}` } });
    await tokenCard.getByRole("button", { name: "VS Code" }).click();
    const mintedVscode = (await tokenCard.getByRole("link", { name: "Add to VS Code" }).getAttribute("href")) ?? "";
    expect(vscodeConfig(mintedVscode)).toEqual({ name: "maidan", type: "http", url: `${origin}/mcp/streamable`, headers: { Authorization: `Bearer ${SECRET}` } });
    await tokenCard.getByRole("button", { name: "Claude Code" }).click();
    await expect(tokenCard.locator(".snippet-body")).toHaveText(`claude mcp add --transport http maidan ${origin}/mcp/streamable --header "Authorization: Bearer ${SECRET}"`);
    await tokenCard.getByRole("button", { name: "Your own loop" }).click();
    await expect(tokenCard.locator(".snippet-body").last()).toContainText(fx.board_channel_id);
    expect(await sideways(page)).toBeLessThanOrEqual(0);
    expect(await shortTargets(page)).toEqual([]);

    await tokenCard.getByRole("button", { name: "I have it" }).click();
    await expect(page.getByText(SECRET)).toHaveCount(0);
    await expect(tokenCard.getByLabel("Agent name")).toBeVisible();
    const kept = await page.evaluate((secret) => {
      const values = (store: Storage) => [...Array(store.length).keys()].map((i) => store.getItem(store.key(i) ?? "") ?? "");
      return {
        dom: document.documentElement.outerHTML.includes(secret),
        local: values(localStorage).some((v) => v.includes(secret)),
        session: values(sessionStorage).some((v) => v.includes(secret)),
      };
    }, SECRET);
    expect(kept).toEqual({ dom: false, local: false, session: false });

    await expect(tokenCard.getByText("REPLACE_WITH_MAIDAN_TOKEN")).toHaveCount(0);
    await tokenCard.getByRole("button", { name: "Already have a token?" }).click();
    await expect(tokenCard.getByRole("button", { name: "Already have a token?" })).toHaveAttribute("aria-expanded", "true");
    await expect(tokenCard.getByText("Replace REPLACE_WITH_MAIDAN_TOKEN with your token.")).toBeVisible();
    await expect(tokenCard.getByText("REPLACE_WITH_MAIDAN_TOKEN").first()).toBeVisible();
    await expect(tokenCard.locator(".snippet-body").first()).toContainText("REPLACE_WITH_MAIDAN_TOKEN");
    expect(await placeholderHrefs(page)).toEqual([]);

    const axe = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    const bad = axe.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
    expect(bad.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).slice(0, 4).join(" | ")}`)).toEqual([]);
    expect(await sideways(page)).toBeLessThanOrEqual(0);
    expect(await shortTargets(page)).toEqual([]);
  });
});
