import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

// Register an MCP tool command, see it in the list, then revoke it. An http
// handler needs an encryption key the harness does not have, so this uses
// mcp_tool, which the server accepts with no secret.
test("slash commands register and revoke from the page", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="slash"]');

  const name = `ping${Date.now().toString(36)}`.slice(0, 32);
  await page.fill("#slash-name", name);
  await page.selectOption("#slash-kind", "mcp_tool");
  await page.fill("#slash-target", "whoami");
  await page.click("#slash-register");

  const list = page.locator("#slash-list");
  await expect(page.locator("#toasts .toast-success", { hasText: `Registered /${name}` })).toHaveAttribute("role", "status");
  await expect(list).toContainText(`/${name}`);
  await expect(list).toContainText("whoami");

  await list.getByRole("button", { name: "Revoke" }).click();
  await expect(page.locator("#toasts .toast-success", { hasText: "Command revoked" })).toHaveAttribute("role", "status");
  await expect(list).not.toContainText(`/${name}`);
});
