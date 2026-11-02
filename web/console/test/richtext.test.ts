import assert from "node:assert/strict";
import { test } from "node:test";
import { parseBlocks, parseInline } from "../src/core/richtext.ts";

test("message text: splits fenced code from prose and keeps the code verbatim", () => {
    const b = parseBlocks("Repro:\n\n```sh\ncargo test -- --nocapture\n  indented\n```\nDone.");
    assert.deepEqual(b, [
      { kind: "para", parts: [{ kind: "text", text: "Repro:" }] },
      { kind: "code", lang: "sh", text: "cargo test -- --nocapture\n  indented" },
      { kind: "para", parts: [{ kind: "text", text: "Done." }] },
    ]);
  });
test("message text: runs an unclosed fence to the end", () => {
    assert.deepEqual(parseBlocks("```\nlet x = 1;"), [{ kind: "code", lang: "", text: "let x = 1;" }]);
  });
test("message text: finds links without the sentence's trailing punctuation", () => {
    assert.deepEqual(parseInline("See https://example.com/a#L1-L2. Then `x` and **y**"), [
      { kind: "text", text: "See " },
      { kind: "link", href: "https://example.com/a#L1-L2", text: "https://example.com/a#L1-L2" },
      { kind: "text", text: ". Then " },
      { kind: "code", text: "x" },
      { kind: "text", text: " and " },
      { kind: "bold", text: "y" },
    ]);
  });
test("message text: never links a non-http scheme", () => {
    assert.deepEqual(parseInline("javascript:alert(1)"), [{ kind: "text", text: "javascript:alert(1)" }]);
  });


test("message text: a URL keeps a closing parenthesis that balances one inside it", () => {
  assert.deepEqual(parseInline("(see https://en.wikipedia.org/wiki/Function_(mathematics))"), [
    { kind: "text", text: "(see " },
    { kind: "link", href: "https://en.wikipedia.org/wiki/Function_(mathematics)", text: "https://en.wikipedia.org/wiki/Function_(mathematics)" },
    { kind: "text", text: ")" },
  ]);
});
