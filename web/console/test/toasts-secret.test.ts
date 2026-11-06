import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";
import esbuild from "esbuild";
import { Window } from "happy-dom";
import { act, createElement } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import type { Notice } from "../src/core/notices.ts";

const require = createRequire(import.meta.url);

// node:test strips types but not JSX. esbuild writes the bundle outside this
// package; react stays the same module the test imports.
async function loadTsx(path: string): Promise<Record<string, unknown>> {
  const dir = mkdtempSync(join(tmpdir(), "maidan-tsx-"));
  const file = join(dir, "mod.mjs");
  const react = pathToFileURL(require.resolve("react")).href;
  const jsx = pathToFileURL(require.resolve("react/jsx-runtime")).href;
  await esbuild.build({
    entryPoints: [path],
    outfile: file,
    bundle: true,
    format: "esm",
    platform: "node",
    jsx: "automatic",
    absWorkingDir: dirname(path),
    plugins: [
      {
        name: "react-abs",
        setup(build) {
          build.onResolve({ filter: /^react$/ }, () => ({ path: react, external: true }));
          build.onResolve({ filter: /^react\/jsx-runtime$/ }, () => ({ path: jsx, external: true }));
        },
      },
    ],
  });
  return import(pathToFileURL(file).href).finally(() => rmSync(dir, { recursive: true, force: true }));
}

const toasts = await loadTsx(new URL("../src/shell/Toasts.tsx", import.meta.url).pathname);
const secret = await loadTsx(new URL("../src/tools/SecretOnce.tsx", import.meta.url).pathname);
const { TOAST_GRACE_MS, TOAST_MS, nextToastCredits, selectLiveToasts, toastRole } = toasts as {
  TOAST_GRACE_MS: number;
  TOAST_MS: number;
  nextToastCredits: (credits: ReadonlyMap<number, number>, list: readonly Notice[], start: number, end: number) => ReadonlyMap<number, number>;
  selectLiveToasts: (list: Notice[], gone: ReadonlySet<number>, navAt: number, now: number, credits: ReadonlyMap<number, number>, frozenAt: number | null) => Notice[];
  toastRole: (tone: Notice["tone"]) => "alert" | "status";
};
const { COPIED_FOR_MS, Copy, SecretOnce, secretView } = secret as {
  COPIED_FOR_MS: number;
  Copy: (props: { text: string }) => unknown;
  SecretOnce: (props: { title: string; secret: string; onDone: () => void }) => unknown;
  secretView: (copied: "no" | "yes" | "failed", shown: boolean, secret: string, seen: string) => { copied: "no" | "yes" | "failed"; shown: boolean; seen: string };
};

const notice = (id: number, at: number, tone: Notice["tone"] = "ok"): Notice => ({ id, text: "Pinned", tone, at });
const none = new Map<number, number>();

function withDom(): () => void {
  const win = new Window({ url: "https://console.test/" });
  const g = globalThis as unknown as Record<string, unknown>;
  const prev = {
    window: g.window,
    document: g.document,
    HTMLElement: g.HTMLElement,
    Node: g.Node,
    IS_REACT_ACT_ENVIRONMENT: g.IS_REACT_ACT_ENVIRONMENT,
  };
  g.window = win;
  g.document = win.document;
  g.HTMLElement = win.HTMLElement;
  g.Node = win.Node;
  g.IS_REACT_ACT_ENVIRONMENT = true;
  return () => {
    win.close();
    g.window = prev.window;
    g.document = prev.document;
    g.HTMLElement = prev.HTMLElement;
    g.Node = prev.Node;
    g.IS_REACT_ACT_ENVIRONMENT = prev.IS_REACT_ACT_ENVIRONMENT;
  };
}

test("a new secret clears the copied and shown state of the card that is already open", () => {
  assert.deepEqual(secretView("yes", true, "second-secret", "first-secret"), { copied: "no", shown: false, seen: "second-secret" });
  assert.deepEqual(secretView("yes", true, "same", "same"), { copied: "yes", shown: true, seen: "same" });
  assert.equal(COPIED_FOR_MS, 2500);
});

test("swapping the secret never paints the next value in the clear", async () => {
  const done = withDom();
  try {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    const paint = (value: string) => {
      act(() => {
        flushSync(() => {
          root.render(createElement(SecretOnce, { title: "New token", secret: value, onDone: () => undefined }));
        });
      });
    };
    paint("first-secret");
    const show = [...host.querySelectorAll("button")].find((b) => b.textContent === "Show");
    assert.ok(show);
    act(() => {
      flushSync(() => show.click());
    });
    assert.match(host.querySelector("code")?.textContent ?? "", /first-secret/);
    paint("second-secret");
    const code = host.querySelector("code")?.textContent ?? "";
    assert.doesNotMatch(code, /second-secret/);
    assert.match(code, /•+/);
    assert.equal([...host.querySelectorAll("button")].some((b) => b.textContent === "Show"), true);
    await act(async () => {
      root.unmount();
    });
  } finally {
    done();
  }
});

test("Copy drops Copied when the text changes and clears it after the hold", async () => {
  const done = withDom();
  const clip = {
    writeText: () => Promise.resolve(),
    readText: () => Promise.resolve(""),
  };
  Object.defineProperty(globalThis.navigator, "clipboard", { configurable: true, value: clip });
  try {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    const paint = (text: string) => {
      act(() => {
        flushSync(() => {
          root.render(createElement(Copy, { text }));
        });
      });
    };
    paint("alpha");
    const button = () => host.querySelector("button");
    assert.equal(button()?.textContent, "Copy");
    await act(async () => {
      button()?.click();
    });
    assert.equal(button()?.textContent, "Copied");
    paint("beta");
    assert.equal(button()?.textContent, "Copy");
    await act(async () => {
      button()?.click();
    });
    assert.equal(button()?.textContent, "Copied");
    await act(async () => {
      await new Promise((r) => setTimeout(r, COPIED_FOR_MS + 40));
    });
    assert.equal(button()?.textContent, "Copy");
    await act(async () => {
      root.unmount();
    });
  } finally {
    done();
  }
});

test("the secret card is a region, and only a failed copy is an alert", async () => {
  const html = await hostHtml(createElement(SecretOnce, { title: "New token", secret: "sekret", onDone: () => undefined }));
  assert.match(html, /role="region"/);
  assert.match(html, /aria-label="New token"/);
  assert.doesNotMatch(html, /role="alert"/);
  assert.match(html, /aria-label="The secret, hidden"/);
  assert.match(html, />Copy</);
  assert.match(html, />Show</);
  assert.match(html, />I have it</);
  const src = readFileSync(new URL("../src/tools/SecretOnce.tsx", import.meta.url), "utf8");
  assert.match(src, /visually-hidden/);
  assert.match(src, /role="status" aria-live="polite"/);
  assert.match(src, /role="alert"/);
  assert.doesNotMatch(src, /sr-only/);
  assert.match(src, /COPIED_FOR_MS/);
});

async function hostHtml(node: ReturnType<typeof createElement>): Promise<string> {
  const done = withDom();
  try {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    await act(async () => {
      root.render(node);
    });
    const html = host.innerHTML;
    await act(async () => {
      root.unmount();
    });
    return html;
  } finally {
    done();
  }
}

test("a pause credits only the toasts that were alive, and a later toast still lasts five seconds", () => {
  const first = notice(1, 0);
  const gone = new Set<number>();
  assert.equal(selectLiveToasts([first], gone, 0, 9000, none, 1000).length, 1);
  assert.equal(selectLiveToasts([first], gone, 0, 9000, none, null).length, 0);
  const held = nextToastCredits(none, [first], 1000, 9000);
  assert.equal(held.get(1), 8000);
  assert.equal(selectLiveToasts([first], gone, 0, 9000, held, null).length, 1);
  assert.ok(TOAST_MS - (9000 - (held.get(1) ?? 0)) >= TOAST_GRACE_MS);
  assert.equal(selectLiveToasts([first], gone, 0, 9000 + TOAST_MS, held, null).length, 0);

  const second = notice(2, 10_000);
  const after = nextToastCredits(held, [first, second], 0, 10_000);
  assert.equal(after.get(2) ?? 0, 0);
  assert.equal(after.has(9), false);
  assert.equal(selectLiveToasts([second], gone, 0, 10_000 + TOAST_MS - 1, after, null).length, 1);
  assert.equal(selectLiveToasts([second], gone, 0, 10_000 + TOAST_MS, after, null).length, 0);
  const pruned = nextToastCredits(after, [second], 10_000, 10_000);
  assert.equal(pruned.has(1), false);
});

test("escape, a route change and the tone choose which toasts remain and how they speak", () => {
  const list = [notice(1, 1000, "ok"), notice(2, 1000, "error")];
  const dismissed = selectLiveToasts(list, new Set([1, 2]), 0, 2000, none, null);
  assert.deepEqual(dismissed, []);
  assert.equal(selectLiveToasts(list, new Set(), 1500, 2000, none, null).length, 0);
  assert.equal(toastRole("error"), "alert");
  assert.equal(toastRole("ok"), "status");
  assert.equal(toastRole("info"), "status");
  assert.equal(toastRole("warn"), "status");
});
