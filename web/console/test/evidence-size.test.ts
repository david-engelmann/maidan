import assert from "node:assert/strict";
import { test } from "node:test";
import { formatBytes } from "../src/core/tools.ts";

test("an evidence size never prints 1024.0 of a unit", () => {
  assert.equal(formatBytes(1024 * 1024 - 1), "1.0 MB");
  assert.equal(formatBytes(1024 * 1024 * 1024 - 1), "1.0 GB");
});

test("a size of a gigabyte or more reads in GB, not thousands of MB", () => {
  assert.equal(formatBytes(5 * 1024 ** 3), "5.0 GB");
});

test("small and missing sizes read as before", () => {
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(2048), "2.0 KB");
  assert.equal(formatBytes(null), "");
  assert.equal(formatBytes(-1), "");
});
