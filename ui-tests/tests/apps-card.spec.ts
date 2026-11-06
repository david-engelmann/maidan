import { test, expect, type APIRequestContext, type Page, type FrameLocator } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";

// The inline approval card (Next 17), driven as an MCP Apps host would drive
// it: the View the server serves at ui://maidan/approval-card.html runs in a
// sandboxed iframe, and a fake host bridge on the page answers its JSON-RPC
// over postMessage (SEP-1865, 2026-01-26). Tool calls the card makes go
// through the bridge to the real server, under the operator's plain bearer,
// as a host's would.

const fx = fixtures();
const CARD_URI = "ui://maidan/approval-card.html";

async function mcp(request: APIRequestContext, token: string, method: string, params: object) {
  const res = await request.post(`${fx.base_url}/mcp`, {
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    data: { jsonrpc: "2.0", id: 1, method, params },
  });
  expect(res.ok()).toBeTruthy();
  const body = await res.json();
  expect(body.error, JSON.stringify(body)).toBeUndefined();
  return body.result;
}

async function cardHtml(request: APIRequestContext): Promise<string> {
  const read = await mcp(request, fx.token, "resources/read", { uri: CARD_URI });
  expect(read.contents[0].mimeType).toBe("text/html;profile=mcp-app");
  return read.contents[0].text as string;
}

async function openGate(request: APIRequestContext, prompt: string): Promise<string> {
  const result = await mcp(request, fx.requester_token, "tools/call", {
    name: "request_approval",
    arguments: { prompt },
  });
  return JSON.parse(result.content[0].text).gate_id as string;
}

// The fake host. `initialResult` is the tool result the host hands the View
// after the handshake. Every request the View makes is kept in
// window.__requests; tools/call goes to the real server through
// window.__callTool (exposed from Node), ui/open-link is recorded only.
async function mountCard(
  page: Page,
  html: string,
  initialResult: object,
  initialArgs: object,
  frame: { width?: number; sandbox?: string } = {},
) {
  const csp: string[] = [];
  page.on("console", (msg) => {
    if (/Content Security Policy/i.test(msg.text())) csp.push(msg.text());
  });
  const width = frame.width ?? 640;
  const sandbox = frame.sandbox ?? "allow-scripts";
  await page.setContent(`<!doctype html><html lang="en"><head><title>Card host</title></head><body><iframe id="card" sandbox="${sandbox}" title="card" style="width:${width}px;height:720px;border:0"></iframe></body></html>`);
  await page.evaluate(
    ({ html, initialResult, initialArgs }) => {
      const w = window as any;
      w.__requests = [];
      w.__opened = [];
      const frame = document.getElementById("card") as HTMLIFrameElement;
      const reply = (id: unknown, result: unknown) =>
        frame.contentWindow!.postMessage({ jsonrpc: "2.0", id, result }, "*");
      const notify = (method: string, params: unknown) =>
        frame.contentWindow!.postMessage({ jsonrpc: "2.0", method, params }, "*");
      window.addEventListener("message", async (event) => {
        if (event.source !== frame.contentWindow) return;
        const msg = event.data;
        if (!msg || msg.jsonrpc !== "2.0" || !msg.method) return;
        w.__requests.push({ method: msg.method, params: msg.params });
        if (msg.method === "ui/initialize") {
          reply(msg.id, {
            protocolVersion: "2026-01-26",
            hostInfo: { name: "fake-host", version: "0" },
            hostCapabilities: { openLinks: {}, serverTools: {} },
            hostContext: { displayMode: "inline" },
          });
        } else if (msg.method === "ui/notifications/initialized") {
          notify("ui/notifications/tool-input", { arguments: initialArgs });
          notify("ui/notifications/tool-result", initialResult);
        } else if (msg.method === "tools/call") {
          const result = await w.__callTool(msg.params);
          reply(msg.id, result);
        } else if (msg.method === "ui/open-link") {
          w.__opened.push(msg.params.url);
          reply(msg.id, {});
        }
      });
      frame.srcdoc = html;
    },
    { html, initialResult, initialArgs },
  );
  return { card: page.frameLocator("#card"), csp };
}

async function exposeServer(page: Page, request: APIRequestContext) {
  await page.exposeFunction("__callTool", async (params: { name: string; arguments: object }) => {
    return await mcp(request, fx.token, "tools/call", params);
  });
}

function requests(page: Page): Promise<Array<{ method: string; params: any }>> {
  return page.evaluate(() => (window as any).__requests);
}

test("the card renders the gate, the evidence tiers and the self-reported warning", async ({ page, request }) => {
  const html = await cardHtml(request);
  await exposeServer(page, request);
  const gate = {
    id: "00000000-0000-7000-8000-000000000001",
    workspace_id: fx.workspace_id,
    requested_by: fx.requester_id,
    prompt: "Deploy v9 to prod? <img src=x onerror=alert(1)>",
    state: "pending",
    risk: "high",
    created_at: new Date().toISOString(),
  };
  const result = {
    content: [{ type: "text", text: JSON.stringify(gate) }],
    isError: false,
    structuredContent: {
      kind: "maidan.approval_gate",
      gate,
      requester: { id: fx.requester_id, handle: "deployer", display_name: "Deployer", kind: "agent" },
      review: {
        evidence_root: "abc",
        self_reported_only: true,
        attestations: [
          { kind: "result", tier: "self_reported", attested_by: "deployer" },
          { kind: "artifact", tier: "attached", sha256: "f".repeat(64), attested_by: "deployer" },
          { kind: "land_gate", tier: "verified", attested_by: "maidan" },
        ],
      },
    },
  };
  const { card, csp } = await mountCard(page, html, result, { gate_id: gate.id });
  await expect(card.locator("#prompt")).toHaveText(gate.prompt);
  // Data is text, never markup.
  await expect(card.locator("#prompt img")).toHaveCount(0);
  await expect(card.locator("#requester")).toContainText("Deployer");
  await expect(card.locator("#requester")).toContainText("agent");
  await expect(card.locator("#risk")).toHaveText("high");
  await expect(card.locator("#state")).toHaveText("Waiting for a decision");
  await expect(card.locator("#self-reported-warning")).toBeVisible();
  await expect(card.locator("#self-reported-warning")).toContainText("self-reported");
  const tiers = card.locator("#evidence .tier");
  await expect(tiers).toHaveText(["self-reported", "attached", "verified"]);
  await expect(card.locator("#accept")).toBeVisible();
  await expect(card.locator("#decline")).toBeVisible();

  const sent = await requests(page);
  expect(sent[0].method).toBe("ui/initialize");
  expect(sent[0].params.protocolVersion).toBe("2026-01-26");
  expect(sent.some((r) => r.method === "ui/notifications/initialized")).toBeTruthy();
  // The pinned CSP admits the card's own script and style and nothing else.
  expect(csp, csp.join("\n")).toEqual([]);
});

test("Decline calls approval_decide through the bridge with the note", async ({ page, request }) => {
  const prompt = `Card decline ${Date.now()}?`;
  const gateId = await openGate(request, prompt);
  const html = await cardHtml(request);
  await exposeServer(page, request);
  const initial = await mcp(request, fx.token, "tools/call", {
    name: "get_approval_gate",
    arguments: { gate_id: gateId },
  });
  expect(initial.structuredContent.kind).toBe("maidan.approval_gate");
  const { card } = await mountCard(page, html, initial, { gate_id: gateId });
  await expect(card.locator("#prompt")).toHaveText(prompt);
  await card.locator("#note").fill("not this week");
  await card.locator("#decline").click();
  await expect(card.locator("#state")).toHaveText("Declined");
  await expect(card.locator("#decide")).toBeHidden();

  const calls = (await requests(page)).filter((r) => r.method === "tools/call");
  const decide = calls.find((c) => c.params.name === "approval_decide");
  expect(decide!.params.arguments).toEqual({ gate_id: gateId, decision: "decline", note: "not this week" });
  // The server agrees: the gate is declined, with the note.
  const after = await mcp(request, fx.token, "tools/call", {
    name: "get_approval_gate",
    arguments: { gate_id: gateId },
  });
  const stored = JSON.parse(after.content[0].text);
  expect(stored.state).toBe("declined");
  expect(stored.content).toEqual({ note: "not this week" });
});

test("Accept that needs confirmation shows the console link and never says it was accepted", async ({ page, request }) => {
  const prompt = `Card accept ${Date.now()}?`;
  const gateId = await openGate(request, prompt);
  const html = await cardHtml(request);
  await exposeServer(page, request);
  const initial = await mcp(request, fx.token, "tools/call", {
    name: "get_approval_gate",
    arguments: { gate_id: gateId },
  });
  const { card } = await mountCard(page, html, initial, { gate_id: gateId });
  await expect(card.locator("#prompt")).toHaveText(prompt);
  await card.locator("#accept").click();

  await expect(card.locator("#confirm")).toBeVisible();
  await expect(card.locator("#confirm-url")).toContainText(`#confirm-approval=${gateId}.`);
  await expect(card.locator("#state")).toHaveText("Waiting for a decision");
  await expect(card.locator("#card")).not.toContainText(/approved/i);
  await expect(card.locator("#state")).not.toHaveText("Accepted");

  await card.locator("#open-link").click();
  await expect
    .poll(() => page.evaluate(() => (window as any).__opened as string[]))
    .toEqual([expect.stringContaining(`#confirm-approval=${gateId}.`)]);

  // Opening the link confirmed nothing: only the person, in the console, can.
  const after = await mcp(request, fx.token, "tools/call", {
    name: "get_approval_gate",
    arguments: { gate_id: gateId },
  });
  expect(JSON.parse(after.content[0].text).state).toBe("pending");
  const calls = (await requests(page)).filter((r) => r.method === "tools/call");
  expect(calls.map((c) => [c.params.name, c.params.arguments.decision])).toContainEqual(["approval_decide", "accept"]);

  // Leave no pending gate behind for the specs that read Needs you.
  await mcp(request, fx.token, "tools/call", {
    name: "approval_decide",
    arguments: { gate_id: gateId, decision: "decline" },
  });
});

async function controlSize(card: FrameLocator, id: string): Promise<string> {
  const box = await card.locator(id).boundingBox();
  expect(box, id).not.toBeNull();
  return `${id} ${Math.round(box!.width)}x${Math.round(box!.height)}`;
}

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, colorScheme: "light" });

  test("the card meets touch size, contrast, focus, and both color schemes", async ({ page, request }) => {
    const html = await cardHtml(request);
    const gate = {
      id: "00000000-0000-7000-8000-0000000000aa",
      workspace_id: fx.workspace_id,
      requested_by: fx.requester_id,
      prompt: "Ship the build?",
      state: "pending",
      risk: "high",
      created_at: new Date().toISOString(),
    };
    const result = {
      content: [{ type: "text", text: "confirmation required" }],
      isError: false,
      structuredContent: {
        kind: "maidan.approval_decision",
        status: "confirmation_required",
        confirmation_url: "/ui/#confirm-approval=relative",
        gate_id: gate.id,
        gate,
      },
    };
    const { card, csp } = await mountCard(page, html, result, { gate_id: gate.id }, {
      width: 358,
      sandbox: "allow-scripts allow-same-origin",
    });
    await expect(card.locator("#accept")).toBeVisible();
    await expect(card.locator("#open-link")).toBeDisabled();
    const fits = await card.locator("html").evaluate((el) => el.scrollWidth <= el.clientWidth);
    expect(fits).toBe(true);
    const small = [];
    for (const id of ["#accept", "#decline", "#refresh", "#open-link"]) {
      const size = await controlSize(card, id);
      const [, dims] = size.split(" ");
      const [w, h] = dims.split("x").map(Number);
      if (w < 44 || h < 44) small.push(size);
    }
    expect(small).toEqual([]);

    // A decision with no requester makes the card refresh. With no tool answer
    // the actions stay disabled, which is the in-flight state. The note is
    // still in the tab order.
    await page.locator("#card").click({ position: { x: 8, y: 8 } });
    await page.keyboard.press("Tab");
    await expect(card.locator("#note")).toBeFocused();
    const outline = await card.locator("#note").evaluate((el) => getComputedStyle(el).outlineStyle);
    expect(outline).not.toBe("none");

    for (const scheme of ["light", "dark"] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      const ratio = await card.locator("#accept, #open-link").evaluateAll((els) => {
        const channel = (c: number) => {
          const s = c / 255;
          return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
        };
        const lum = (rgb: number[]) => 0.2126 * channel(rgb[0]) + 0.7152 * channel(rgb[1]) + 0.0722 * channel(rgb[2]);
        const parse = (color: string) => {
          const m = color.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?\)/);
          if (!m) return null;
          return { rgb: [Number(m[1]), Number(m[2]), Number(m[3])], a: m[4] === undefined ? 1 : Number(m[4]) };
        };
        const one = (el: Element) => {
          const fg = parse(getComputedStyle(el).color);
          let node: Element | null = el;
          let bg: number[] | null = null;
          while (node) {
            const parsed = parse(getComputedStyle(node).backgroundColor);
            if (parsed && parsed.a > 0) {
              bg = parsed.rgb;
              break;
            }
            node = node.parentElement;
          }
          if (!fg || !bg) return 0;
          const hi = Math.max(lum(fg.rgb), lum(bg));
          const lo = Math.min(lum(fg.rgb), lum(bg));
          return (hi + 0.05) / (lo + 0.05);
        };
        return Math.min(...els.map(one));
      });
      expect(ratio, scheme).toBeGreaterThanOrEqual(4.5);
      const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
      expect(scan.violations.map((v) => `${scheme} ${v.id}`), scheme).toEqual([]);
    }
    expect(csp, csp.join("\n")).toEqual([]);
  });
});
