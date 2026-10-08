import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools } from "./_tools";

const fx = fixtures();

// A model, holding the operator's plain bearer (no approval:grant), asks to
// accept a gate. The answer is the confirmation link, not an acceptance.
async function askToAccept(request: APIRequestContext, gateId: string) {
  const mcp = await request.post(`${fx.base_url}/mcp`, {
    headers: { Authorization: `Bearer ${fx.token}`, "Content-Type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "approval_decide",
        arguments: { gate_id: gateId, decision: "accept" },
      },
    },
  });
  expect(mcp.ok()).toBeTruthy();
  const body = await mcp.json();
  expect(body.error, JSON.stringify(body)).toBeUndefined();
  const text = body.result.content[0].text as string;
  return JSON.parse(text) as { status: string; confirmation_url?: string };
}

async function signIn(page: Page) {
  await page.context().addCookies([
    { name: "maidan_session", value: fx.session_cookie, url: fx.base_url, httpOnly: true, sameSite: "Lax" },
  ]);
}

async function openGate(request: APIRequestContext, prompt: string) {
  const mcp = await request.post(`${fx.base_url}/mcp`, {
    headers: { Authorization: `Bearer ${fx.requester_token}`, "Content-Type": "application/json" },
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
  const body = await mcp.json();
  const text = body.result.content[0].text as string;
  return (JSON.parse(text).gate_id as string);
}

test("a model's accept through approval_decide asks the signed-in person to confirm", async ({ page, request }) => {
  const prompt = `Confirm deploy ${Date.now()}?`;
  const gateId = await openGate(request, prompt);
  const asked = await askToAccept(request, gateId);
  expect(asked.status).toBe("confirmation_required");
  expect(asked.confirmation_url).toContain(`#confirm-approval=${gateId}.`);

  await signIn(page);
  const confirms: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "POST" && req.url().includes("/approval-confirmations/confirm")) {
      confirms.push(req.url());
    }
  });
  await page.goto(asked.confirmation_url!);
  await expect(page.locator("#identity-mode")).toHaveText("session");

  const panel = page.locator("#approval-list li.confirmation");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText(prompt);
  await expect(panel.locator(".model-request")).toContainText("a model asked via an unidentified MCP client");
  // The token lived in the fragment and the page clears it before rendering.
  await expect(page).not.toHaveURL(/confirm-approval/);

  await panel.locator("button.gate-confirm").click();
  await expect(panel).toContainText("was accepted");
  await expect(panel.locator(".model-request")).toContainText("decided via an unidentified MCP client, requested by a model");
  expect(confirms.length).toBe(1);
  expect(confirms[0]).toContain("/ui/api/approval-confirmations/confirm");

  // The gate left the pending list.
  await expect(page.locator("#approval-list li.approval-row").filter({ hasText: prompt })).toHaveCount(0);

  // The link is spent: opening it again confirms nothing.
  await page.goto(asked.confirmation_url!);
  const again = page.locator("#approval-list li.confirmation").last();
  await expect(again).toBeVisible();
  await again.locator("button.gate-confirm").click();
  await expect(page.locator("#toasts [role=alert]")).toBeVisible();
});
