import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parseBlocks, parseInline } from "../src/core/richtext.ts";

const textOf = (parts: { kind: string; text?: string }[]) => parts.map((p) => (p.kind === "break" ? "\n" : p.text ?? "")).join("");

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
    for (const raw of ["[click](javascript:alert(1))", "[x](data:text/html,hi)", "[x](vbscript:msg)", "[x](file:///tmp/a)", "[x](./local)", "[x](/threads/1)"]) {
      const parts = parseInline(raw);
      assert.equal(parts.some((p) => p.kind === "link"), false, raw);
      assert.equal(textOf(parts), raw);
    }
  });

test("message text: splits blank lines into paragraphs and keeps a single newline", () => {
  assert.deepEqual(parseBlocks("First.\n\nSecond."), [
    { kind: "para", parts: [{ kind: "text", text: "First." }] },
    { kind: "para", parts: [{ kind: "text", text: "Second." }] },
  ]);
  assert.deepEqual(parseBlocks("First.\nSecond."), [
    { kind: "para", parts: [{ kind: "text", text: "First." }, { kind: "break" }, { kind: "text", text: "Second." }] },
  ]);
});

test("message text: headings, lists, quotes, rules and tables stay structured", () => {
  const blocks = parseBlocks([
    "# Title",
    "## Section",
    "- **bold** and *italic*",
    "- plain",
    "3. third",
    "4. fourth",
    "> Quoted *word*",
    ">",
    "> Next.",
    "---",
    "| Col | Val |",
    "| --- | --- |",
    "| a | b |",
    "***",
  ].join("\n"));
  assert.deepEqual(blocks.map((b) => b.kind), ["heading", "heading", "ul", "ol", "quote", "hr", "table", "hr"]);
  assert.deepEqual(blocks[0], { kind: "heading", level: 1, parts: [{ kind: "text", text: "Title" }] });
  assert.equal(blocks[1].kind === "heading" && blocks[1].level, 2);
  assert.deepEqual(blocks[2].kind === "ul" ? blocks[2].items : [], [
    [{ kind: "bold", text: "bold" }, { kind: "text", text: " and " }, { kind: "italic", text: "italic" }],
    [{ kind: "text", text: "plain" }],
  ]);
  assert.deepEqual(blocks[3].kind === "ol" ? [blocks[3].start, blocks[3].items] : [], [3, [[{ kind: "text", text: "third" }], [{ kind: "text", text: "fourth" }]]]);
  assert.equal(blocks[4].kind, "quote");
  if (blocks[4].kind === "quote") {
    assert.equal(blocks[4].blocks[0].kind, "para");
    assert.equal(blocks[4].blocks[1].kind, "para");
  }
  assert.deepEqual(blocks[6].kind === "table" ? blocks[6].header.map((c) => c[0]) : [], [
    { kind: "text", text: "Col" },
    { kind: "text", text: "Val" },
  ]);
});

test("message text: a rule is not a table row, and a tilde fence keeps its language", () => {
  const blocks = parseBlocks("| Col | Val |\n---\n~~~js\nconst x = 1;\n~~~");
  assert.equal(blocks[0].kind, "para");
  assert.equal(blocks[1].kind, "hr");
  assert.deepEqual(blocks[2], { kind: "code", lang: "js", text: "const x = 1;" });
});

test("message text: a route name stops at a word boundary", () => {
  for (const raw of ["#/needsX", "#/people-ops", "#/threadsfoo"]) {
    assert.deepEqual(parseInline(raw), [{ kind: "text", text: raw }]);
  }
  assert.deepEqual(parseInline("#/needs"), [{ kind: "link", href: "#/needs", text: "#/needs", internal: true }]);
  assert.deepEqual(parseInline("(#/people)"), [
    { kind: "text", text: "(" },
    { kind: "link", href: "#/people", text: "#/people", internal: true },
    { kind: "text", text: ")" },
  ]);
  assert.deepEqual(parseInline("#/threads/t/th_1"), [
    { kind: "link", href: "#/threads/t/th_1", text: "#/threads/t/th_1", internal: true },
  ]);
  assert.deepEqual(parseInline("#/board?channel=ch_1"), [
    { kind: "link", href: "#/board?channel=ch_1", text: "#/board?channel=ch_1", internal: true },
  ]);
});

test("message text: links allow http, https and console routes, and code stays literal", () => {
  assert.deepEqual(parseInline("[PR #42](https://example.com/a_(b))"), [
    { kind: "link", href: "https://example.com/a_(b)", text: "PR #42" },
  ]);
  assert.deepEqual(parseInline("[Open task](#/needs)"), [
    { kind: "link", href: "#/needs", text: "Open task", internal: true },
  ]);
  assert.deepEqual(parseInline("See #/threads/t/abc. and #/not-a-route"), [
    { kind: "text", text: "See " },
    { kind: "link", href: "#/threads/t/abc", text: "#/threads/t/abc", internal: true },
    { kind: "text", text: ". and #/not-a-route" },
  ]);
  assert.deepEqual(parseInline("snake_case and a*b*c and `https://x.com *z*` and ~~gone~~"), [
    { kind: "text", text: "snake_case and a*b*c and " },
    { kind: "code", text: "https://x.com *z*" },
    { kind: "text", text: " and " },
    { kind: "strike", text: "gone" },
  ]);
});

test("the message view maps blocks to elements and never injects HTML", () => {
  const src = readFileSync(new URL("../src/threads/MessageBody.tsx", import.meta.url), "utf8");
  const css = readFileSync(new URL("../src/threads/richtext.css", import.meta.url), "utf8");
  assert.match(src, /import "\.\/richtext\.css"/);
  assert.doesNotMatch(src, /dangerouslySetInnerHTML/);
  assert.doesNotMatch(src, /<h1|<h2/);
  assert.match(src, /aria-label="Copy code"/);
  assert.match(src, /className="msg-copy"/);
  assert.match(src, /target="_blank" rel="noopener noreferrer"/);
  assert.match(src, /startsWith\("#\/"\)/);
  for (const name of ["msg-rich", "msg-heading", "msg-h3", "msg-h4", "msg-h5", "msg-h6", "msg-list", "msg-quote", "msg-rule", "msg-table-wrap", "msg-table", "msg-em", "msg-del", "msg-codeblock", "msg-codehead", "msg-lang", "msg-copy"]) {
    assert.match(src, new RegExp(name), name);
    assert.match(css, new RegExp(`\\.${name}\\b`), name);
  }
});

