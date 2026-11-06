import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";
import ts from "typescript";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { Notice } from "../src/core/notices.ts";

// node:test strips types but not JSX, so the components are transpiled here
// and loaded from inside this package, where `react` resolves.
function loadTsx(path: string): Promise<Record<string, unknown>> {
  const src = readFileSync(path, "utf8").replaceAll('from "../core/', 'from "../../src/core/');
  const js = ts.transpileModule(src, {
    compilerOptions: { jsx: ts.JsxEmit.ReactJSX, module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    fileName: path,
  }).outputText;
  const dir = mkdtempSync(join(new URL(".", import.meta.url).pathname, ".tsx-load-"));
  const file = join(dir, "mod.mjs");
  writeFileSync(file, js);
  return import(pathToFileURL(file).href).finally(() => rmSync(dir, { recursive: true, force: true }));
}

const toasts = await loadTsx(new URL("../src/shell/Toasts.tsx", import.meta.url).pathname);
const secret = await loadTsx(new URL("../src/tools/SecretOnce.tsx", import.meta.url).pathname);
const { TOAST_GRACE_MS, TOAST_MS, pauseCredit, selectLiveToasts, toastRole } = toasts as {
  TOAST_GRACE_MS: number;
  TOAST_MS: number;
  pauseCredit: (credit: number, spent: number, now: number, list: Notice[], navAt: number, gone: ReadonlySet<number>) => number;
  selectLiveToasts: (list: Notice[], gone: ReadonlySet<number>, navAt: number, now: number, credit: number, frozenAt: number | null) => Notice[];
  toastRole: (tone: Notice["tone"]) => "alert" | "status";
};
const { COPIED_FOR_MS, SecretOnce, secretView } = secret as {
  COPIED_FOR_MS: number;
  SecretOnce: (props: { title: string; secret: string; onDone: () => void }) => unknown;
  secretView: (copied: "no" | "yes" | "failed", shown: boolean, secret: string, seen: string) => { copied: "no" | "yes" | "failed"; shown: boolean; seen: string };
};

const notice = (id: number, at: number, tone: Notice["tone"] = "ok"): Notice => ({ id, text: "Pinned", tone, at });

test("a new secret clears the copied and shown state of the card that is already open", () => {
  assert.deepEqual(secretView("yes", true, "second-secret", "first-secret"), { copied: "no", shown: false, seen: "second-secret" });
  assert.deepEqual(secretView("yes", true, "same", "same"), { copied: "yes", shown: true, seen: "same" });
  assert.equal(COPIED_FOR_MS, 2500);
});

test("the secret card is a region, and only a failed copy is an alert", () => {
  const html = renderToStaticMarkup(createElement(SecretOnce, { title: "New token", secret: "sekret", onDone: () => undefined }));
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

test("hover and focus freeze a toast, and leaving gives back the paused time", () => {
  const list = [notice(1, 0)];
  const gone = new Set<number>();
  assert.equal(selectLiveToasts(list, gone, 0, 9000, 0, 1000).length, 1);
  assert.equal(selectLiveToasts(list, gone, 0, 9000, 0, null).length, 0);
  const credit = pauseCredit(0, 8000, 9000, list, 0, gone);
  assert.equal(selectLiveToasts(list, gone, 0, 9000, credit, null).length, 1);
  assert.ok(TOAST_MS - (9000 - credit) >= TOAST_GRACE_MS);
  assert.equal(selectLiveToasts(list, gone, 0, 9000 + TOAST_MS, credit, null).length, 0);
});

test("escape, a route change and the tone choose which toasts remain and how they speak", () => {
  const list = [notice(1, 1000, "ok"), notice(2, 1000, "error")];
  const dismissed = selectLiveToasts(list, new Set([1, 2]), 0, 2000, 0, null);
  assert.deepEqual(dismissed, []);
  assert.equal(selectLiveToasts(list, new Set(), 1500, 2000, 0, null).length, 0);
  assert.equal(toastRole("error"), "alert");
  assert.equal(toastRole("ok"), "status");
  assert.equal(toastRole("info"), "status");
  assert.equal(toastRole("warn"), "status");
});