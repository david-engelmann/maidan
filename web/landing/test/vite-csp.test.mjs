import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const src = readFileSync(join(root, "vite.config.ts"), "utf8");

test("landing build disables asset inlining for strict CSP", () => {
  assert.match(src, /assetsInlineLimit:\s*0/);
});

test("landing keeps stable embeddable filenames", () => {
  assert.match(src, /entryFileNames:\s*"app\.js"/);
  assert.match(src, /return "style\.css"/);
});
