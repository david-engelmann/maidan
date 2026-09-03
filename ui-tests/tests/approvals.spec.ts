import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// The human side of the held gate, in a real browser: an agent opens a gate,
// it appears in the Approvals tab, and Accept resolves it. The gate is created
// per-run with a unique prompt so the spec is retry-safe and independent of any
// other pending gate.
test("the Approvals tab lists a pending gate and resolves it on Accept", async ({ page, request }) => {
  const prompt = `Ship build ${Date.now()}?`;
  const mcp = await request.post(`${fx.base_url}/mcp`, {
    headers: { Authorization: `Bearer ${fx.token}`, "Content-Type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "request_approval",
        arguments: { prompt, thread_id: fx.thread_id },
      },
    },
  });
  expect(mcp.ok()).toBeTruthy();

  await page.goto("/ui/");
  // Authenticate + point at the seeded workspace before opening the tab (it
  // loads on click, reading #workspace).
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click('.tabs button[data-tab="approvals"]');

  // Our pending gate renders with its three actions.
  const row = page.locator("#approval-list li.approval-row").filter({ hasText: prompt });
  await expect(row).toBeVisible();
  await expect(row.locator("button.gate-accept")).toBeVisible();
  await expect(row.locator("button.gate-decline")).toBeVisible();
  await expect(row.locator("button.gate-cancel")).toBeVisible();

  // Accept it → the gate resolves (CAS on pending) and leaves the pending list.
  await row.locator("button.gate-accept").click();
  await expect(row).toHaveCount(0);
});
