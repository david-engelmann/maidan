import { test } from "node:test";
import assert from "node:assert/strict";
import { START_RETRY_SECS, createTryPage, refusal } from "../try/try.js";
import { UNAVAILABLE_RETRY_SECS } from "../src/edge.js";

const CAP_DETAIL = "the demo is full until its daily reset; try again later";
const STARTING_DETAIL = `The server is starting or being updated. Retry in ${UNAVAILABLE_RETRY_SECS} seconds.`;
const FULL = "The demo is full until its daily reset. Try again later.";
const STARTING = `The demo is starting or being updated. Try again in about ${START_RETRY_SECS} seconds.`;

const minted = {
  agent: { token: "mdn_agent" },
  admin: { token: "mdn_admin" },
  mcp_url: "https://try.maidan.world/mcp/streamable",
};

function classList() {
  const names = new Set();
  return {
    toggle(name, on) {
      if (on) names.add(name);
      else names.delete(name);
    },
    add(name) {
      names.add(name);
    },
    remove(name) {
      names.delete(name);
    },
    has(name) {
      return names.has(name);
    },
  };
}

function element(id) {
  return {
    id,
    hidden: id === "turnstile" || id === "ready",
    disabled: false,
    textContent: "",
    href: "",
    dataset: {},
    classList: classList(),
    focus() {
      this.focused = true;
    },
    addEventListener(_type, fn) {
      this.listeners ??= [];
      this.listeners.push(fn);
    },
  };
}

function page() {
  const ids = [
    "turnstile",
    "start",
    "start-status",
    "agent-token",
    "claude-cmd",
    "codex-config",
    "cursor-link",
    "vscode-link",
    "ready",
    "ready-title",
    "console",
    "console-status",
  ];
  const elements = new Map(ids.map((id) => [id, element(id)]));
  const doc = { getElementById: (id) => elements.get(id) };
  return { doc, elements, el: (id) => elements.get(id) };
}

function problem(status, detail, { type = "https://maidan.world/problems/overloaded", retryAfter } = {}) {
  const headers = { "content-type": "application/problem+json" };
  if (retryAfter !== undefined) headers["retry-after"] = String(retryAfter);
  return new Response(JSON.stringify({ type, title: "Service Unavailable", status, detail }), { status, headers });
}

function harness(handler) {
  const ui = page();
  const calls = [];
  const assigned = [];
  const fetch = async (url, init) => {
    calls.push({ url, init });
    return handler(url, init, calls);
  };
  const app = createTryPage(ui.doc, fetch, (href) => assigned.push(href));
  return { ...ui, app, calls, assigned };
}

test("the page's retry hint is the Worker's Retry-After", () => {
  assert.equal(START_RETRY_SECS, UNAVAILABLE_RETRY_SECS);
});

test("refusal keeps the daily cap for a sandbox 503 and treats every other 503 as the container starting", () => {
  assert.equal(refusal(503, CAP_DETAIL, { type: "https://maidan.world/problems/overloaded", onSandbox: true }), FULL);
  assert.equal(
    refusal(503, STARTING_DETAIL, {
      type: "https://maidan.world/problems/overloaded",
      retryAfter: "20",
      onSandbox: true,
    }),
    STARTING,
  );
  assert.equal(
    refusal(503, CAP_DETAIL, { type: "https://maidan.world/problems/overloaded", retryAfter: "20", onSandbox: true }),
    STARTING,
    "a Retry-After means the container, even when the detail mentions the reset",
  );
  assert.equal(refusal(503, CAP_DETAIL, { type: "https://maidan.world/problems/overloaded", onSandbox: false }), STARTING);
  assert.equal(refusal(503, "", { onSandbox: false }), STARTING);
  assert.equal(refusal(503, "", { onSandbox: true }), STARTING);
  assert.equal(refusal(429, "slow down", { onSandbox: true }), "You have started the most sandboxes one address may this hour. Try again later.");
  assert.equal(refusal(404, "", { onSandbox: true }), "This demo is not taking new sandboxes right now.");
  assert.equal(refusal(400, "That email is not accepted."), "That email is not accepted.");
  assert.equal(refusal(500, "panic at db.rs:12"), "The sandbox could not be started. Try again.");
  assert.equal(refusal(502, "complete the human check first"), "Complete the human check first.");
  assert.equal(refusal(502, "the upstream failed"), "The sandbox could not be started. Try again.");
});

test("start() says the demo is full only for the sandbox cap", async () => {
  const cap = harness(async () => problem(503, CAP_DETAIL));
  await cap.app.start();
  assert.equal(cap.el("start-status").textContent, FULL);
  assert.equal(cap.el("start-status").classList.has("error"), true);
  assert.equal(cap.el("start").disabled, false);
  assert.equal(cap.el("ready").hidden, true);
  assert.equal(cap.calls[0].url, "/sandbox");
  assert.equal(cap.calls[0].init.method, "POST");

  const waking = harness(async () => problem(503, STARTING_DETAIL, { retryAfter: 20 }));
  await waking.app.start();
  assert.equal(waking.el("start-status").textContent, STARTING);
  assert.equal(waking.el("start").disabled, false);

  const bare = harness(async () => new Response("nope", { status: 503, headers: { "retry-after": "20" } }));
  await bare.app.start();
  assert.equal(bare.el("start-status").textContent, STARTING);
});

test("openConsole() treats every 503 as the demo starting, including a cap-shaped body", async () => {
  async function signIn(response) {
    const ui = harness(async (url) => {
      if (url === "/sandbox") {
        return new Response(JSON.stringify(minted), { status: 201, headers: { "content-type": "application/json" } });
      }
      return response();
    });
    await ui.app.start();
    assert.equal(ui.el("ready").hidden, false);
    assert.equal(ui.el("start").hidden, true);
    await ui.app.openConsole();
    return ui;
  }

  const cap = await signIn(() => problem(503, CAP_DETAIL));
  assert.equal(cap.el("console-status").textContent, STARTING);
  assert.equal(cap.el("console-status").classList.has("error"), true);
  assert.equal(cap.el("console").disabled, false);
  assert.deepEqual(cap.assigned, []);
  const consoleCall = cap.calls.find((call) => call.url === "/auth/session/from-token");
  assert.equal(consoleCall.init.method, "POST");
  assert.equal(consoleCall.init.headers.authorization, "Bearer mdn_admin");

  const waking = await signIn(() => problem(503, STARTING_DETAIL, { retryAfter: 20 }));
  assert.equal(waking.el("console-status").textContent, STARTING);

  const down = await signIn(() => new Response("", { status: 503, headers: { "retry-after": "20" } }));
  assert.equal(down.el("console-status").textContent, STARTING);
});

test("a sandbox that starts still opens the console", async () => {
  const ui = harness(async (url) => {
    if (url === "/sandbox") {
      return new Response(JSON.stringify(minted), { status: 201, headers: { "content-type": "application/json" } });
    }
    return new Response(null, { status: 204 });
  });
  await ui.app.start();
  assert.equal(ui.el("agent-token").textContent, "mdn_agent");
  assert.match(ui.el("claude-cmd").textContent, /mdn_agent/);
  await ui.app.openConsole();
  assert.deepEqual(ui.assigned, ["/ui/"]);
  assert.equal(ui.el("console-status").textContent, "Signing you in…");
});

test("start() still names the hourly limit, a closed demo, and a network failure", async () => {
  const limited = harness(async () => problem(429, "slow down", { type: "https://maidan.world/problems/rate-limited" }));
  await limited.app.start();
  assert.equal(
    limited.el("start-status").textContent,
    "You have started the most sandboxes one address may this hour. Try again later.",
  );

  const closed = harness(async () =>
    problem(404, "no", { type: "https://maidan.world/problems/not-found" }),
  );
  await closed.app.start();
  assert.equal(closed.el("start-status").textContent, "This demo is not taking new sandboxes right now.");

  const offline = harness(async () => {
    throw new Error("offline");
  });
  await offline.app.start();
  assert.equal(offline.el("start-status").textContent, "Could not reach the demo. Check your connection and try again.");
  assert.equal(offline.el("start").disabled, false);
});

test("a second copy click replaces the reset timer instead of restoring Copy early", async () => {
  const ui = page();
  ui.el("agent-token").textContent = "mdn_agent";
  const button = element("copy-token");
  button.dataset.copy = "agent-token";
  const listeners = [];
  button.addEventListener = (_type, fn) => listeners.push(fn);
  const scheduled = [];
  const realSet = globalThis.setTimeout;
  const realClear = globalThis.clearTimeout;
  globalThis.setTimeout = (fn, ms) => {
    const id = scheduled.length + 1;
    scheduled.push({ id, fn, ms, cleared: false });
    return id;
  };
  globalThis.clearTimeout = (id) => {
    const timer = scheduled.find((item) => item.id === id);
    if (timer) timer.cleared = true;
  };
  const clipboard = globalThis.navigator?.clipboard;
  Object.defineProperty(globalThis.navigator, "clipboard", {
    configurable: true,
    value: { writeText: async () => {} },
  });
  ui.doc.querySelectorAll = () => [button];
  ui.doc.head = { appendChild() {} };
  ui.doc.createElement = () => ({});
  try {
    createTryPage(ui.doc, async () => new Response("{}", { status: 200 })).mount();
    await listeners[0]();
    await listeners[0]();
    assert.equal(button.textContent, "Copied");
    assert.equal(scheduled.length, 2);
    assert.equal(scheduled[0].cleared, true);
    assert.equal(scheduled[1].cleared, false);
    scheduled[1].fn();
    assert.equal(button.textContent, "Copy");
  } finally {
    globalThis.setTimeout = realSet;
    globalThis.clearTimeout = realClear;
    if (clipboard) Object.defineProperty(globalThis.navigator, "clipboard", { configurable: true, value: clipboard });
  }
});

test("a failed human check stays hidden and the start button tries again", async () => {
  const ui = page();
  const scripts = [];
  ui.doc.head = { appendChild(script) { scripts.push(script); } };
  ui.doc.createElement = () => ({ remove() { this.removed = true; } });
  ui.doc.querySelectorAll = () => [];
  const previous = globalThis.window;
  globalThis.window = {};
  let configCalls = 0;
  const app = createTryPage(ui.doc, async (url) => {
    if (url === "/try-config.json") {
      configCalls += 1;
      return new Response(JSON.stringify({ turnstile_site_key: "site-key" }), { status: 200 });
    }
    return new Response(JSON.stringify(minted), { status: 201, headers: { "content-type": "application/json" } });
  });
  try {
    const flush = () => new Promise((resolve) => setTimeout(resolve, 10));
    app.mount();
    await flush();
    assert.equal(ui.el("turnstile").hidden, true);
    scripts[0].onerror();
    await flush();
    assert.equal(scripts[0].removed, true);
    assert.equal(ui.el("turnstile").hidden, true);
    assert.match(ui.el("start-status").textContent, /did not load/);
    const pending = app.start();
    await flush();
    globalThis.window.turnstile = {
      render(_el, opts) {
        opts.callback("tok");
        return "widget-1";
      },
    };
    scripts.at(-1).onload();
    await pending;
    assert.equal(configCalls, 2);
    assert.equal(ui.el("ready").hidden, false);
    assert.equal(ui.el("agent-token").textContent, "mdn_agent");
  } finally {
    globalThis.window = previous;
  }
});
