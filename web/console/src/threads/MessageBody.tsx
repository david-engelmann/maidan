import { parseBlocks, type Inline } from "../core/richtext.ts";

function Part({ p }: { p: Inline }) {
  if (p.kind === "code") return <code className="msg-code">{p.text}</code>;
  if (p.kind === "bold") return <strong>{p.text}</strong>;
  if (p.kind === "link") return <a className="msg-link" href={p.href} target="_blank" rel="noopener noreferrer">{p.text}</a>;
  return <>{p.text}</>;
}

/** A message body: paragraphs with inline code, bold and links, and fenced code blocks. */
export function MessageBody({ body }: { body: string }) {
  return (
    <>
      {parseBlocks(body).map((b, i) =>
        b.kind === "code" ? (
          <pre key={i} className="msg-pre" tabIndex={0}><code>{b.text}</code></pre>
        ) : (
          <p key={i} className="msg-para">{b.parts.map((p, j) => <Part key={j} p={p} />)}</p>
        ),
      )}
    </>
  );
}
