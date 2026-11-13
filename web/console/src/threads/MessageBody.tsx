import { useState } from "react";
import { parseBlocks, type Block, type Inline } from "../core/richtext.ts";
import "./richtext.css";

function Part({ p }: { p: Inline }) {
  if (p.kind === "break") return <br />;
  if (p.kind === "code") return <code className="msg-code">{p.text}</code>;
  if (p.kind === "bold") return <strong>{p.text}</strong>;
  if (p.kind === "italic") return <em className="msg-em">{p.text}</em>;
  if (p.kind === "strike") return <del className="msg-del">{p.text}</del>;
  if (p.kind === "link") {
    // A console route stays in this tab. Anything else was already limited to http(s).
    if (p.internal || p.href.startsWith("#/")) return <a className="msg-link" href={p.href}>{p.text}</a>;
    return <a className="msg-link" href={p.href} target="_blank" rel="noopener noreferrer">{p.text}</a>;
  }
  return <>{p.text}</>;
}

function Parts({ parts }: { parts: Inline[] }) {
  return <>{parts.map((p, i) => <Part key={i} p={p} />)}</>;
}

function CopyCode({ text }: { text: string }) {
  const [copied, setCopied] = useState<"idle" | "copied" | "failed">("idle");
  const copy = () => {
    let writing: Promise<void> | undefined;
    try {
      writing = navigator.clipboard?.writeText(text);
    } catch {
      setCopied("failed");
      return;
    }
    if (!writing) {
      setCopied("failed");
      return;
    }
    void writing.then(() => setCopied("copied"), () => setCopied("failed"));
  };
  return (
    <>
      {/* The name is the visible word plus hidden context, never an aria-label
          over it: "Copied" under a "Copy code" label is a name a voice-control
          user cannot say (WCAG 2.5.3). */}
      <button type="button" className="msg-copy" onClick={(e) => { e.stopPropagation(); copy(); }}>
        {copied === "copied" ? "Copied" : "Copy"}
        <span className="visually-hidden"> code</span>
      </button>
      {copied === "failed" ? <p className="err small" role="alert">Could not copy.</p> : null}
    </>
  );
}

function CodeBlock({ lang, text }: { lang: string; text: string }) {
  return (
    <div className="msg-codeblock">
      <div className="msg-codehead">
        {lang ? <span className="msg-lang">{lang}</span> : null}
        <CopyCode text={text} />
      </div>
      <pre className="msg-pre" tabIndex={0}><code>{text}</code></pre>
    </div>
  );
}

// The shell and the thread pane already use h1 and h2. Shift this message so
// its shallowest heading is h3, then let each later heading rise by at most
// one level. A skipped markdown level (`#` then `###`) must not skip h4.
// @heading-ranks
export function clampHeadingLevel(level: number, min: number, state: { last: number }): number {
  const raw = Math.min(6, Math.max(3, level - min + 3));
  const clamped = Math.min(raw, state.last + 1);
  state.last = clamped;
  return clamped;
}

function shallowest(blocks: Block[]): number {
  let min = 7;
  for (const block of blocks) {
    if (block.kind === "heading") min = Math.min(min, block.level);
    else if (block.kind === "quote") min = Math.min(min, shallowest(block.blocks));
  }
  return min;
}

function walkHeadings(blocks: Block[], min: number, state: { last: number }, out: number[]) {
  for (const block of blocks) {
    if (block.kind === "heading") out.push(clampHeadingLevel(block.level, min, state));
    else if (block.kind === "quote") walkHeadings(block.blocks, min, state, out);
  }
}

/** Rendered heading ranks for a body, in document order, quotes included. */
export function headingRanks(body: string): number[] {
  const blocks = parseBlocks(body);
  const out: number[] = [];
  walkHeadings(blocks, shallowest(blocks), { last: 2 }, out);
  return out;
}
// @heading-ranks-end

function headingClass(level: number): { tag: "h3" | "h4" | "h5" | "h6"; className: string } {
  if (level <= 3) return { tag: "h3", className: "msg-heading msg-h3" };
  if (level === 4) return { tag: "h4", className: "msg-heading msg-h4" };
  if (level === 5) return { tag: "h5", className: "msg-heading msg-h5" };
  return { tag: "h6", className: "msg-heading msg-h6" };
}

function Blocks({ blocks, min, ranks }: { blocks: Block[]; min: number; ranks: { last: number } }) {
  return blocks.map((block, i) => {
    if (block.kind === "code") return <CodeBlock key={i} lang={block.lang} text={block.text} />;
    if (block.kind === "heading") {
      const { tag: Tag, className } = headingClass(clampHeadingLevel(block.level, min, ranks));
      return <Tag key={i} className={className}><Parts parts={block.parts} /></Tag>;
    }
    if (block.kind === "ul") {
      return (
        <ul key={i} className="msg-list">
          {block.items.map((item, j) => <li key={j}><Parts parts={item} /></li>)}
        </ul>
      );
    }
    if (block.kind === "ol") {
      return (
        <ol key={i} className="msg-list" start={block.start}>
          {block.items.map((item, j) => <li key={j}><Parts parts={item} /></li>)}
        </ol>
      );
    }
    if (block.kind === "quote") {
      return (
        <blockquote key={i} className="msg-quote">
          <Blocks blocks={block.blocks} min={min} ranks={ranks} />
        </blockquote>
      );
    }
    if (block.kind === "hr") return <hr key={i} className="msg-rule" />;
    if (block.kind === "table") {
      return (
        // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- a wide table scrolls, so its wrapper takes keyboard focus.
        <div key={i} className="msg-table-wrap" tabIndex={0}>
          <table className="msg-table">
            <thead>
              <tr>{block.header.map((cell, j) => <th key={j}><Parts parts={cell} /></th>)}</tr>
            </thead>
            <tbody>
              {block.rows.map((row, r) => (
                <tr key={r}>{row.map((cell, c) => <td key={c}><Parts parts={cell} /></td>)}</tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }
    return <p key={i} className="msg-para"><Parts parts={block.parts} /></p>;
  });
}

/** A message body: paragraphs, lists, tables, links and fenced code. */
export function MessageBody({ body }: { body: string }) {
  const blocks = parseBlocks(body);
  return (
    <div className="msg-rich">
      <Blocks blocks={blocks} min={shallowest(blocks)} ranks={{ last: 2 }} />
    </div>
  );
}
