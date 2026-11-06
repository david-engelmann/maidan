import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const src = readFileSync(join(root, "vite.config.ts"), "utf8");

test("console keeps assetsInlineLimit at 0", () => {
  assert.match(src, /assetsInlineLimit:\s*0/);
});

test("console proxy targets stay on loopback", () => {
  const targets = [...src.matchAll(/https?:\/\/([^:/\"']+)/g)].map((m) => m[1]);
  const ws = [...src.matchAll(/ws:\/\/([^:/\"']+)/g)].map((m) => m[1]);
  for (const host of [...targets, ...ws]) {
    assert.equal(host, "127.0.0.1", `non-loopback proxy host: ${host}`);
  }
});

test("console enables fs.strict and strictPort for local dev", () => {
  assert.match(src, /strictPort:\s*true/);
  assert.match(src, /fs:\s*\{\s*strict:\s*true\s*\}/);
});
