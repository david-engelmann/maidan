import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { sheet } from "./sheet.ts";

// Files scrolled 173px sideways at 900px: the "Choose a task" select took its
// longest option's width, and neither its field's grid track nor the select
// itself could shrink below it. Both now shrink with the card.
const css = sheet().replace(/\/\*[\s\S]*?\*\//g, "");

test("a growing row-form field's track and control shrink with the card", () => {
  assert.match(css, /\.row-form \.field\.grow \{[^}]*grid-template-columns: minmax\(0, 1fr\);/);
  assert.match(css, /\.row-form \.field\.grow select, \.row-form \.field\.grow input \{[^}]*width: 100%; min-width: 0;/);
});

test("Files still puts its task picker in a growing row-form field", () => {
  const src = readFileSync(new URL("../src/artifacts/Artifacts.tsx", import.meta.url), "utf8");
  assert.match(src, /<form className="row-form"[\s\S]*?<label className="field grow">Choose a task\s*<select/);
});
