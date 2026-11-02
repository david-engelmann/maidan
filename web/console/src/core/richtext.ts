// Message bodies as plain data: fenced code blocks, inline code, bold and
// bare links. No HTML is ever produced from the text, so nothing in a message
// can inject markup; the view maps these pieces to React elements.

export type Inline =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "bold"; text: string }
  | { kind: "link"; href: string; text: string };

export type Block = { kind: "para"; parts: Inline[] } | { kind: "code"; lang: string; text: string };

// Quote characters are written as \x escapes so source scanners that pair quotes stay in step.
const FENCE = /^\x60{3}([\w+-]*)[ \t]*$/;

/** Splits a body into paragraphs and fenced code blocks. An unclosed fence runs to the end. */
export function parseBlocks(body: string): Block[] {
  const out: Block[] = [];
  const lines = body.replace(/\r\n?/g, "\n").split("\n");
  let para: string[] = [];
  const flush = () => {
    const text = para.join("\n").replace(/^\n+|\n+$/g, "");
    if (text) out.push({ kind: "para", parts: parseInline(text) });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const open = FENCE.exec(lines[i].trim());
    if (!open) { para.push(lines[i]); continue; }
    flush();
    const code: string[] = [];
    for (i++; i < lines.length && lines[i].trim() !== "```"; i++) code.push(lines[i]);
    out.push({ kind: "code", lang: open[1], text: code.join("\n") });
  }
  flush();
  return out;
}

// A link ends before trailing punctuation a sentence would put after it.
const TOKEN = /(\x60[^\x60\n]+\x60)|(\*\*[^*\n]+\*\*)|(https?:\/\/[^\s<>\x22\x27\x60]+[^\s<>\x22\x27\x60.,;:!?)\]])/g;

export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  let last = 0;
  for (const m of text.matchAll(TOKEN)) {
    const at = m.index ?? 0;
    if (at > last) out.push({ kind: "text", text: text.slice(last, at) });
    if (m[1]) out.push({ kind: "code", text: m[1].slice(1, -1) });
    else if (m[2]) out.push({ kind: "bold", text: m[2].slice(2, -2) });
    else out.push({ kind: "link", href: m[3], text: m[3] });
    last = at + m[0].length;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out;
}
