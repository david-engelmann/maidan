import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools } from "./_tools";

const fx = fixtures();

// A model, holding the operator's plain bearer (no approval:grant), asks to
// accept a gate. The answer is the confirmation link, not an acceptance.
// `clientName` sends it as a 2026-07-28 client's self-reported clientInfo.
async function askToAccept(request: APIRequestContext, gateId: string, clientName?: string) {
  const headers: Record<string, string> = { Authorization: `Bearer ${fx.token}`, "Content-Type": "application/json" };
  const params: Record<string, unknown> = {
    name: "approval_decide",
    arguments: { gate_id: gateId, decision: "accept" },
  };
  if (clientName) {
    params._meta = {
      "io.modelcontextprotocol/protocolVersion": "2026-07-28",
      "io.modelcontextprotocol/clientCapabilities": {},
      "io.modelcontextprotocol/clientInfo": { name: clientName, version: "1.0" },
    };
    headers["mcp-protocol-version"] = "2026-07-28";
    headers["mcp-method"] = "tools/call";
    headers["mcp-name"] = "approval_decide";
  }
  const mcp = await request.post(`${fx.base_url}/mcp`, {
    headers,
    data: { jsonrpc: "2.0", id: 1, method: "tools/call", params },
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
  expect(confirms[0]).toContain("/auth/approval-confirmations/confirm");

  // The gate left the pending list.
  await expect(page.locator("#approval-list li.approval-row:not(.confirmation)").filter({ hasText: prompt })).toHaveCount(0);

  // The link is spent: opening it again confirms nothing.
  await page.goto(asked.confirmation_url!);
  const again = page.locator("#approval-list li.confirmation").last();
  await expect(again).toBeVisible();
  await expect(again).toContainText("no longer pending");
  await expect(again.locator("button.gate-confirm")).toHaveCount(0);
});

test("a self-reported client is marked so on the confirmation card", async ({ page, request }) => {
  const prompt = `Confirm self-reported ${Date.now()}?`;
  const gateId = await openGate(request, prompt);
  const asked = await askToAccept(request, gateId, "ChatGPT");
  expect(asked.status).toBe("confirmation_required");

  await signIn(page);
  await page.goto(asked.confirmation_url!);
  const panel = page.locator("#approval-list li.confirmation");
  await expect(panel).toContainText(prompt);
  await expect(panel.locator(".model-request")).toContainText("a model asked via ChatGPT 1.0 (self-reported)");
  await panel.locator("button.gate-confirm").click();
  await expect(panel).toContainText("was accepted");
  await expect(panel.locator(".model-request")).toContainText(
    "decided via ChatGPT 1.0 (self-reported), requested by a model",
  );
});

test("the console names a credential client plainly, a self-reported one as such, and none as unidentified", async ({ page }) => {
  await signIn(page);
  await page.goto(`${fx.base_url}/ui/`);
  // A string, so the test runner's transpiler leaves the browser's
  // dynamic import alone.
  const lines = (await page.evaluate(`(async () => {
    const tools = await import("/ui/static/tools.js");
    const gate = (via) => ({ decided_via: Object.assign({ model_asked: true }, via) });
    return {
      credential: tools.decidedViaLine(gate({ client_name: "Release Bot", client_id: "a1", client_source: "credential" })),
      selfReported: tools.decidedViaLine(gate({ client_name: "ChatGPT", client_version: "1.0", client_source: "self_reported" })),
      none: tools.decidedViaLine(gate({ client_source: "none" })),
      legacy: tools.decidedViaLine(gate({ client_name: "Old Client" })),
      request: tools.modelRequestLine({ client_name: "Release Bot", client_source: "credential" }),
    };
  })()`)) as Record<string, string>;
  expect(lines.credential).toBe("decided via Release Bot, requested by a model");
  expect(lines.selfReported).toBe("decided via ChatGPT 1.0 (self-reported), requested by a model");
  expect(lines.none).toBe("decided via an unidentified MCP client, requested by a model");
  expect(lines.legacy).toBe("decided via Old Client (self-reported), requested by a model");
  expect(lines.request).toBe("a model asked via Release Bot, waiting for you to confirm");
});
