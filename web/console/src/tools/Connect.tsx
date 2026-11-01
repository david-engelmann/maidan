// Connect an agent to this workspace over MCP, and mint the first admin
// token for this session. Snippets carry a placeholder, never a token.
import { useState } from "react";
import { cursorDeeplink, llmsTxt, mcpUrl, snippets } from "../core/connect.ts";
import type { ToolsApi } from "../core/tools.ts";
import { SecretOnce } from "./SecretOnce.tsx";

function Copy({ text }: { text: string }) {
  const [said, setSaid] = useState<string | null>(null);
  return (
    <button
      type="button"
      className="button ghost"
      onClick={() => navigator.clipboard.writeText(text).then(() => setSaid("Copied"), () => setSaid("Copy failed"))}
    >
      {said ?? "Copy"}
    </button>
  );
}

export function Connect({ api, base, signedIn }: { api: ToolsApi; base: string; signedIn: boolean }) {
  const [secret, setSecret] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const mint = async () => {
    setBusy(true);
    setNote(null);
    try {
      const out = await api.mintAdmin();
      setSecret(out.secret);
    } catch (e) {
      setNote(e instanceof Error ? e.message : "Could not mint a token");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <section className="card">
        <h3>Connect an agent</h3>
        <p className="muted small">Paste one into your agent, then swap in a token from People.</p>
        <p className="mono-id break">{mcpUrl(base)}</p>
        {snippets(base).map((s) => (
          <div key={s.id} className="snippet">
            <div className="snippet-head"><strong>{s.title}</strong><Copy text={s.text} /></div>
            <div className="snippet-scroll"><pre className="snippet-body">{s.text}</pre></div>
          </div>
        ))}
        <div className="edit-actions">
          <a className="button steel" href={cursorDeeplink(base)}>Add to Cursor</a>
          <a className="button ghost" href={llmsTxt(base)} target="_blank" rel="noreferrer">Read llms.txt</a>
        </div>
      </section>
      {signedIn ? (
        <section className="card">
          <h3>First admin token</h3>
          <p className="muted small">A new workspace has no token yet. Mint one for your session; it is shown once.</p>
          <button type="button" className="button steel" disabled={busy} onClick={() => void mint()}>{busy ? "Minting…" : "Mint an admin token"}</button>
          {note ? <p className="err small" role="alert">{note}</p> : null}
          {secret ? <SecretOnce title="Admin token" secret={secret} onDone={() => setSecret(null)} /> : null}
        </section>
      ) : null}
    </>
  );
}
