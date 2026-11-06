import assert from "node:assert/strict";
import { rmSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";
import { Window } from "happy-dom";
import { createElement, useState, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { act } from "react";
import { NeedsApi, parseConfirmLink, sortNeeds, type WaitingItem } from "../src/core/needs.ts";
import { leaveRoom, redact } from "../src/core/notices.ts";
import { liveWord } from "../src/core/firstRun.ts";
import { LIST_ROWS, isTyping } from "../src/shell/commands.ts";
import { navigate, targetHash } from "../src/shell/router.ts";
import { useLoad } from "../src/hooks/useTools.ts";
import type { Credentials } from "../src/core/api.ts";
import type { ScreenBoundary as Boundary, roomKeyOf as RoomKey } from "../src/App.tsx";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const SHA = "ab".repeat(32);
const UUID = "123e4567-e89b-12d3-a456-426614174000";
const creds = (): Credentials => ({ base: "https://m.example", pastedToken: "tok", tokenSession: false, sessionMemberId: "m1" });
const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json", ...headers } });

test("redact hides mdn tokens and bearer secrets, and keeps ids", () => {
  assert.equal(redact("token mdn_ab"), "token [hidden]");
  assert.equal(redact("mdn_Zx9.short"), "[hidden]");
  assert.equal(redact(`made mdn_${"Zx9".repeat(12)}`), "made [hidden]");
  assert.equal(redact("Authorization: Bearer maid_secret, next"), "Authorization: Bearer [hidden], next");
  assert.equal(redact("Bearer abc"), "Bearer [hidden]");
  assert.equal(redact(UUID), UUID);
  assert.equal(redact(SHA), SHA);
  assert.equal(redact(`see ${SHA} then Bearer ${SHA}`), `see ${SHA} then Bearer [hidden]`);
});

test("a review packet refusal is words, and a blank confirm link is not a link", async () => {
  const api = new NeedsApi(async () => json(403, { error: "no" }), creds, () => "bearer");
  const denied = await api.reviewPacket("t");
  assert.equal(denied.state, "error");
  if (denied.state === "error") {
    assert.match(denied.why, /^Could not load the evidence\. /);
    assert.equal(/\b(403|429|500)\b/.test(denied.why), false);
    assert.match(denied.why, /not allowed/);
  }
  const limited = new NeedsApi(async () => json(429, { error: "slow" }, { "Retry-After": "30" }), creds, () => "bearer");
  const wait = await limited.reviewPacket("t");
  assert.equal(wait.state, "error");
  if (wait.state === "error") {
    assert.match(wait.why, /Too many requests/);
    assert.equal(wait.why.includes("429"), false);
  }
  const broken = new NeedsApi(async () => json(500, { error: "panic at db.rs:12" }), creds, () => "bearer");
  const down = await broken.reviewPacket("t");
  assert.equal(down.state, "error");
  if (down.state === "error") {
    assert.match(down.why, /server hit an error/);
    assert.equal(down.why.includes("500"), false);
    assert.equal(down.why.includes("db.rs"), false);
  }
  const row = (since: string): WaitingItem => ({ kind: "open_gate", gate_id: since, summary: "s", since, age_secs: 1 });
  const sorted = sortNeeds([row("not-a-date"), row("2026-10-30T09:00:00Z"), row("also-bad")]);
  assert.equal(sorted[0]?.gate_id, "2026-10-30T09:00:00Z");
  assert.equal(parseConfirmLink("#confirm-approval=%20.%20"), null);
  assert.equal(parseConfirmLink("#/confirm-approval=%20.token"), null);
  assert.equal(parseConfirmLink("#confirm-approval=gate.%20"), null);
  assert.deepEqual(parseConfirmLink("#confirm-approval=gate.token"), { gateId: "gate", token: "token" });
});

test("the live pill says Connecting until the socket is up", () => {
  assert.equal(liveWord("connecting", false), "Connecting");
  assert.equal(liveWord("connecting", true), "Signed out");
  assert.equal(liveWord("offline", false), "Offline");
});

test("shortcuts ignore fields, editors, and a modal dialog", () => {
  assert.equal(isTyping({ tagName: "INPUT" } as unknown as EventTarget), true);
  assert.equal(isTyping({ tagName: "input" } as unknown as EventTarget), true);
  assert.equal(isTyping({ tagName: "DIV", isContentEditable: true } as unknown as EventTarget), true);
  assert.equal(isTyping({ tagName: "BUTTON" } as unknown as EventTarget), false);
  assert.equal(isTyping(null), false);
  const closest = (hit: string) => (selector: string) => (selector.includes(hit) ? {} : null);
  assert.equal(isTyping({ tagName: "SPAN", closest: closest("contenteditable") } as unknown as EventTarget), true);
  assert.equal(isTyping({ tagName: "SPAN", closest: closest("textbox") } as unknown as EventTarget), true);
  assert.equal(isTyping({ tagName: "BUTTON", closest: closest("aria-modal") } as unknown as EventTarget), true);
  assert.equal(isTyping({ tagName: "BUTTON", closest: () => null } as unknown as EventTarget), false);
  assert.match(LIST_ROWS, /\.act-list \.act-link/);
  assert.match(LIST_ROWS, /\.activity-list a/);
  assert.match(LIST_ROWS, /\.cat-item > summary/);
  assert.match(LIST_ROWS, /\.tool-row/);
});

test("going to a place drops its sub-route", () => {
  assert.equal(targetHash("tools"), "#/tools");
  let current = "#/tools/prefs";
  const loc = {};
  Object.defineProperty(loc, "hash", {
    get: () => current,
    set: (value: string) => {
      current = value.startsWith("#") ? value : `#${value}`;
    },
  });
  const previous = globalThis.window;
  globalThis.window = { location: loc } as unknown as Window & typeof globalThis.window;
  try {
    navigate("tools");
    assert.equal(current, "#/tools");
    navigate("tools");
    assert.equal(current, "#/tools");
    current = "#/people/m/mem_1";
    navigate("people");
    assert.equal(current, "#/people");
    current = "#/threads/t/th_1";
    navigate("threads");
    assert.equal(current, "#/threads");
    current = "#/board?task=t_1";
    navigate("board");
    assert.equal(current, "#/board");
  } finally {
    globalThis.window = previous;
  }
});

async function boundaryModule(): Promise<{ ScreenBoundary: typeof Boundary; roomKeyOf: typeof RoomKey }> {
  const require = createRequire(import.meta.url);
  const esbuild = require("esbuild") as typeof import("esbuild");
  const outfile = path.join(import.meta.dirname, ".tmp-screen-boundary.mjs");
  await esbuild.build({
    entryPoints: [path.join(import.meta.dirname, "../src/App.tsx")],
    bundle: true,
    format: "esm",
    outfile,
    jsx: "automatic",
    external: ["react", "react-dom", "react/jsx-runtime", "react/jsx-dev-runtime"],
    loader: { ".css": "empty" },
  });
  try {
    return await import(pathToFileURL(outfile).href) as { ScreenBoundary: typeof Boundary; roomKeyOf: typeof RoomKey };
  } finally {
    rmSync(outfile, { force: true });
  }
}

async function withDom<T>(run: () => Promise<T> | T): Promise<T> {
  const dom = new Window();
  const previous = { window: globalThis.window, document: globalThis.document };
  globalThis.window = dom as unknown as Window & typeof globalThis.window;
  globalThis.document = dom.document as unknown as Document;
  try {
    return await run();
  } finally {
    globalThis.window = previous.window;
    globalThis.document = previous.document;
  }
}

function mount(ui: ReactNode): { host: HTMLDivElement; root: Root } {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => {
    root.render(ui);
  });
  return { host, root };
}

test("a screen recovers from a throw, and a room change clears it", async () => {
  const { ScreenBoundary, roomKeyOf } = await boundaryModule();
  assert.equal(roomKeyOf({ status: "signed-in", workspaceId: "ws", memberId: "mem" }), "ws:mem");
  assert.equal(roomKeyOf({ status: "signed-in", workspaceId: "ws", memberId: null }), "ws:");
  assert.equal(roomKeyOf({ status: "signed-out", workspaceId: null, memberId: null }), null);
  assert.equal(leaveRoom("ws:mem", null), true);
  assert.equal(leaveRoom("ws:mem", "ws:other"), true);
  assert.equal(leaveRoom(null, "ws:mem"), false);
  assert.equal(leaveRoom("ws:mem", "ws:mem"), false);

  await withDom(() => {
    function Child({ crash }: { crash: boolean }) {
      if (crash) throw new Error("boom");
      return createElement("p", null, "board is here");
    }
    function Harness() {
      const [key, setKey] = useState("board:a");
      const [crash, setCrash] = useState(false);
      return createElement(
        "div",
        null,
        createElement("button", { type: "button", onClick: () => setCrash(true) }, "break"),
        createElement("button", { type: "button", onClick: () => setCrash(false) }, "heal"),
        createElement("button", { type: "button", onClick: () => { setCrash(false); setKey("board:b"); } }, "switch"),
        createElement(ScreenBoundary, { resetKey: key }, createElement(Child, { crash })),
      );
    }
    const { host, root } = mount(createElement(Harness));
    assert.match(host.textContent ?? "", /board is here/);
    const click = (label: string) => {
      const button = [...host.querySelectorAll("button")].find((b) => b.textContent === label);
      act(() => {
        button?.click();
      });
    };
    click("break");
    assert.match(host.textContent ?? "", /unexpected error/);
    click("heal");
    assert.match(host.textContent ?? "", /unexpected error/);
    click("Try again");
    assert.match(host.textContent ?? "", /board is here/);
    click("break");
    assert.match(host.textContent ?? "", /unexpected error/);
    click("switch");
    assert.match(host.textContent ?? "", /board is here/);
    assert.equal((host.textContent ?? "").includes("unexpected error"), false);
    act(() => root.unmount());
  });
});

test("useLoad ignores a read that finishes after its key is cleared", async () => {
  let release: (value: string) => void = () => {};
  let started = false;
  const read = () => {
    started = true;
    return new Promise<string>((resolve) => {
      release = resolve;
    });
  };
  function Probe() {
    const [key, setKey] = useState<string | null>("sha");
    const loaded = useLoad(key, read);
    return createElement(
      "div",
      null,
      createElement("button", { type: "button", onClick: () => setKey(null) }, "clear"),
      createElement("output", null, `${loaded.status}:${loaded.value ?? ""}`),
    );
  }
  await withDom(async () => {
    const { host, root } = mount(createElement(Probe));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    assert.equal(started, true);
    act(() => {
      (host.querySelector("button") as HTMLButtonElement | null)?.click();
    });
    await act(async () => {
      release("secret-bytes");
      await Promise.resolve();
    });
    assert.equal((host.querySelector("output")?.textContent ?? "").includes("secret-bytes"), false);
    act(() => root.unmount());
  });
});
