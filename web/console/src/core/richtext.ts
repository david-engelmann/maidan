// Message bodies as plain data. Fenced code, headings, lists, quotes, tables
// and inline marks become pieces the view maps to React elements. No HTML is
// ever produced from the text, so nothing in a message can inject markup.
//
// A link is emitted only for http(s) or a console route (`#/...`). Any other
// scheme stays text: a message must not be able to navigate the browser to
// `javascript:` or `data:`.

export type Inline =
  | { kind: "text"; text: string }
  | { kind: "break" }
  | { kind: "code"; text: string }
  | { kind: "bold"; text: string }
  | { kind: "italic"; text: string }
  | { kind: "strike"; text: string }
  | { kind: "link"; href: string; text: string; internal?: boolean };

export type Block =
  | { kind: "para"; parts: Inline[] }
  | { kind: "code"; lang: string; text: string }
  | { kind: "heading"; level: number; parts: Inline[] }
  | { kind: "ul"; items: Inline[][] }
  | { kind: "ol"; start: number; items: Inline[][] }
  | { kind: "quote"; blocks: Block[] }
  | { kind: "hr" }
  | { kind: "table"; header: Inline[][]; rows: Inline[][][] };

// Quote characters are written as \x escapes so source scanners that pair quotes stay in step.
const FENCE = /^(`{3,}|~{3,})([\w+-]*)[ \t]*$/;
const HEADING = /^(#{1,6})[ \t]+(.*)$/;
const HR = /^(?:-{3,}|\*{3,}|_{3,})[ \t]*$/;
const UL = /^[ \t]*[-*][ \t]+(.*)$/;
const OL = /^[ \t]*(\d+)\.[ \t]+(.*)$/;
const QUOTE = /^[ \t]*> ?(.*)$/;
// Same shape as before: the last character cannot be sentence punctuation, and
// a later pass restores a `)` the URL itself opened.
const URL = /https?:\/\/[^\s<>\x22\x27\x60]+[^\s<>\x22\x27\x60.,;:!?)\]]/;
const HASH = /^#\/(?:threads|board|needs|artifacts|people|activity|catalog|tools)(?:\/[^\s<>\x22\x27\x60)\]]*)?/;

type Fence = { mark: string; len: number; lang: string };

function openFence(trimmed: string): Fence | null {
  const m = FENCE.exec(trimmed);
  if (!m) return null;
  return { mark: m[1][0], len: m[1].length, lang: m[2] };
}

function closesFence(trimmed: string, fence: Fence): boolean {
  return new RegExp(`^\\${fence.mark}{${fence.len},}[ \\t]*$`).test(trimmed);
}

/** A link the pattern stopped before a `)` keeps that `)` when the link opens
 * one it has not closed, as in `…/Function_(mathematics)`. */
function balanceParen(link: string, after: string): string {
  const open = (link.match(/\(/g) ?? []).length;
  const close = (link.match(/\)/g) ?? []).length;
  return open > close && after.startsWith(")") ? `${link})` : link;
}

function safeHref(href: string): boolean {
  return href.startsWith("#/") || /^https?:\/\//i.test(href);
}

function pushText(out: Inline[], text: string) {
  if (!text) return;
  const prev = out[out.length - 1];
  if (prev?.kind === "text") prev.text += text;
  else out.push({ kind: "text", text });
}

function takeUrl(text: string, at: number): { href: string; length: number } | null {
  const m = URL.exec(text.slice(at));
  if (!m || m.index !== 0) return null;
  const href = balanceParen(m[0], text.slice(at + m[0].length));
  return safeHref(href) ? { href, length: href.length } : null;
}

function takeHash(text: string, at: number): { href: string; length: number } | null {
  if (at > 0 && !/[\s(]/.test(text[at - 1])) return null;
  const m = HASH.exec(text.slice(at));
  if (!m) return null;
  const href = m[0].replace(/[.,;:!?)]+$/, "");
  if (!/^#\/(?:threads|board|needs|artifacts|people|activity|catalog|tools)(?:\/|$)/.test(href)) return null;
  return { href, length: href.length };
}

/** `[label](href)`, with parentheses inside the URL balanced. An unsafe href is not a link. */
function takeMdLink(text: string, at: number): { label: string; href: string; length: number } | null {
  if (text[at] !== "[") return null;
  const labelEnd = text.indexOf("](", at + 1);
  if (labelEnd < 0) return null;
  const label = text.slice(at + 1, labelEnd);
  if (!label || /[\n\]]/.test(label)) return null;
  let depth = 1;
  let i = labelEnd + 2;
  const start = i;
  for (; i < text.length && depth > 0; i++) {
    const c = text[i];
    if (c === "\n" || c === " " || c === "\t") return null;
    if (c === "(") depth++;
    else if (c === ")") depth--;
    if (depth === 0) {
      const href = text.slice(start, i);
      if (!href || !safeHref(href)) return null;
      return { label, href, length: i + 1 - at };
    }
  }
  return null;
}

function takeMarked(text: string, at: number, marker: string): { inner: string; length: number } | null {
  if (!text.startsWith(marker, at)) return null;
  const from = at + marker.length;
  const end = text.indexOf(marker, from);
  if (end < 0) return null;
  const inner = text.slice(from, end);
  if (!inner || inner.includes("\n")) return null;
  if (marker === "**" || marker === "*") {
    if (inner.includes("*")) return null;
  }
  if (marker === "~~" && inner.includes("~")) return null;
  if (marker === "*") {
    const prev = at > 0 ? text[at - 1] : "";
    const next = text[end + 1] ?? "";
    if (/[A-Za-z0-9_]/.test(prev) || /[A-Za-z0-9_]/.test(next)) return null;
    if (inner.startsWith(" ") || inner.endsWith(" ")) return null;
  }
  return { inner, length: end + marker.length - at };
}

function parseLine(text: string): Inline[] {
  const out: Inline[] = [];
  let i = 0;
  while (i < text.length) {
    if (text[i] === "\x60") {
      const marked = takeMarked(text, i, "\x60");
      if (marked) {
        out.push({ kind: "code", text: marked.inner });
        i += marked.length;
        continue;
      }
    }
    const link = text[i] === "[" ? takeMdLink(text, i) : null;
    if (link) {
      const internal = link.href.startsWith("#/");
      out.push(internal ? { kind: "link", href: link.href, text: link.label, internal: true } : { kind: "link", href: link.href, text: link.label });
      i += link.length;
      continue;
    }
    const strike = text.startsWith("~~", i) ? takeMarked(text, i, "~~") : null;
    if (strike) {
      out.push({ kind: "strike", text: strike.inner });
      i += strike.length;
      continue;
    }
    const bold = text.startsWith("**", i) ? takeMarked(text, i, "**") : null;
    if (bold) {
      out.push({ kind: "bold", text: bold.inner });
      i += bold.length;
      continue;
    }
    const italic = text[i] === "*" ? takeMarked(text, i, "*") : null;
    if (italic) {
      out.push({ kind: "italic", text: italic.inner });
      i += italic.length;
      continue;
    }
    const url = text.startsWith("http://", i) || text.startsWith("https://", i) ? takeUrl(text, i) : null;
    if (url) {
      out.push({ kind: "link", href: url.href, text: url.href });
      i += url.length;
      continue;
    }
    const hash = text[i] === "#" ? takeHash(text, i) : null;
    if (hash) {
      out.push({ kind: "link", href: hash.href, text: hash.href, internal: true });
      i += hash.length;
      continue;
    }
    pushText(out, text[i]);
    i++;
  }
  return out;
}

/** Inline marks on one paragraph. A single newline is a break; a blank line is not (that splits blocks). */
export function parseInline(text: string): Inline[] {
  const lines = text.split("\n");
  const out: Inline[] = [];
  lines.forEach((line, index) => {
    if (index) out.push({ kind: "break" });
    out.push(...parseLine(line));
  });
  return out;
}

function cells(line: string): string[] {
  let row = line.trim();
  if (row.startsWith("|")) row = row.slice(1);
  if (row.endsWith("|")) row = row.slice(0, -1);
  return row.split("|").map((cell) => cell.trim());
}

function isDelimiter(line: string): boolean {
  const row = cells(line);
  return row.length > 1 && row.every((cell) => /^:?-{3,}:?$/.test(cell));
}

function isTable(lines: string[], at: number): boolean {
  return at + 1 < lines.length && lines[at].includes("|") && isDelimiter(lines[at + 1]);
}

function headingText(raw: string): { level: number; text: string } | null {
  const m = HEADING.exec(raw.trim());
  if (!m) return null;
  const text = m[2].replace(/\s+#+\s*$/, "").trim();
  if (!text) return null;
  return { level: m[1].length, text };
}

/** Splits a body into blocks. An unclosed fence runs to the end. */
export function parseBlocks(body: string): Block[] {
  const lines = body.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  const para: string[] = [];
  const flush = () => {
    const text = para.join("\n").replace(/^\n+|\n+$/g, "");
    para.length = 0;
    if (text) out.push({ kind: "para", parts: parseInline(text) });
  };
  for (let i = 0; i < lines.length; i++) {
    const trimmed = lines[i].trim();
    const fence = openFence(trimmed);
    if (fence) {
      flush();
      const code: string[] = [];
      for (i++; i < lines.length && !closesFence(lines[i].trim(), fence); i++) code.push(lines[i]);
      out.push({ kind: "code", lang: fence.lang, text: code.join("\n") });
      continue;
    }
    if (trimmed === "") {
      flush();
      continue;
    }
    if (HR.test(trimmed)) {
      flush();
      out.push({ kind: "hr" });
      continue;
    }
    const heading = headingText(lines[i]);
    if (heading) {
      flush();
      out.push({ kind: "heading", level: heading.level, parts: parseInline(heading.text) });
      continue;
    }
    if (isTable(lines, i)) {
      flush();
      const header = cells(lines[i]).map((cell) => parseInline(cell));
      i += 2;
      const rows: Inline[][][] = [];
      for (; i < lines.length && lines[i].trim() !== "" && lines[i].includes("|") && !isDelimiter(lines[i]); i++) {
        rows.push(cells(lines[i]).map((cell) => parseInline(cell)));
      }
      i--;
      out.push({ kind: "table", header, rows });
      continue;
    }
    if (QUOTE.test(lines[i])) {
      flush();
      const quoted: string[] = [];
      for (; i < lines.length && QUOTE.test(lines[i]); i++) quoted.push(QUOTE.exec(lines[i])?.[1] ?? "");
      i--;
      const blocks = parseBlocks(quoted.join("\n"));
      if (blocks.length) out.push({ kind: "quote", blocks });
      continue;
    }
    const bullet = UL.exec(lines[i]);
    if (bullet && !HR.test(trimmed)) {
      flush();
      const items: Inline[][] = [];
      for (; i < lines.length; i++) {
        const item = UL.exec(lines[i]);
        if (!item || HR.test(lines[i].trim())) break;
        items.push(parseInline(item[1]));
      }
      i--;
      out.push({ kind: "ul", items });
      continue;
    }
    const numbered = OL.exec(lines[i]);
    if (numbered) {
      flush();
      const start = Number(numbered[1]);
      const items: Inline[][] = [];
      for (; i < lines.length; i++) {
        const item = OL.exec(lines[i]);
        if (!item) break;
        items.push(parseInline(item[2]));
      }
      i--;
      out.push({ kind: "ol", start, items });
      continue;
    }
    para.push(lines[i]);
  }
  flush();
  return out;
}
