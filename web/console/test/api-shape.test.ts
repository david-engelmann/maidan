import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  MAX_API_LIST,
  MAX_API_STRING,
  isPlainObject,
  optionalString,
  requireApiId,
  requireFiniteNumber,
  requireNonEmptyString,
  requireStringList,
} from "../src/core/api-shape.ts";

test("plain objects only — arrays and null fail", () => {
  assert.equal(isPlainObject({ a: 1 }), true);
  assert.equal(isPlainObject(null), false);
  assert.equal(isPlainObject([]), false);
  assert.equal(isPlainObject("x"), false);
});

test("non-empty strings trim and reject blanks / oversize", () => {
  assert.equal(requireNonEmptyString("  ab  ", "name"), "ab");
  assert.throws(() => requireNonEmptyString("", "name"), /non-empty/);
  assert.throws(() => requireNonEmptyString("   ", "name"), /non-empty/);
  assert.throws(() => requireNonEmptyString(1 as unknown, "name"), /string/);
  assert.throws(() => requireNonEmptyString("x".repeat(MAX_API_STRING + 1), "name"), /exceeds/);
});

test("api ids reject whitespace and path separators", () => {
  assert.equal(requireApiId("thr_1"), "thr_1");
  assert.throws(() => requireApiId("a b"), /path separators|whitespace/);
  assert.throws(() => requireApiId("../x"), /path separators|whitespace/);
  assert.throws(() => requireApiId("a\\b"), /path separators|whitespace/);
});

test("finite numbers only", () => {
  assert.equal(requireFiniteNumber(0), 0);
  assert.throws(() => requireFiniteNumber(Number.NaN), /finite/);
  assert.throws(() => requireFiniteNumber(Number.POSITIVE_INFINITY), /finite/);
  assert.throws(() => requireFiniteNumber("1" as unknown), /finite/);
});

test("string lists are capped and item-checked", () => {
  assert.deepEqual(requireStringList([" a ", "b"]), ["a", "b"]);
  assert.throws(() => requireStringList("x" as unknown), /array/);
  assert.throws(() => requireStringList([""]), /non-empty/);
  assert.throws(() => requireStringList(Array(MAX_API_LIST + 1).fill("a")), /exceeds/);
});

test("optionalString maps blank to null", () => {
  assert.equal(optionalString(null), null);
  assert.equal(optionalString(undefined), null);
  assert.equal(optionalString("  "), null);
  assert.equal(optionalString(" hi "), "hi");
  assert.throws(() => optionalString(1 as unknown), /string or null/);
});

test("api-contract points at the runtime gates module", () => {
  const text = readFileSync(new URL("../src/core/api-contract.ts", import.meta.url), "utf8");
  assert.match(text, /api-shape/);
  assert.match(text, /Fits</);
});
