// Connect an agent to this workspace over MCP, and mint the first admin
// token for this session. Signing in with OAuth comes first; the token
// snippets are the fallback. Snippets carry a placeholder, never a token.
import { useState } from "react";
import { OAUTH_CLIENTS, TOKEN_FALLBACK, TOKEN_PLACEHOLDER, cursorDeeplink, llmsTxt, snippets, type Snippet } from "../core/connect.ts";
import type { ToolsApi } from "../core/tools.ts";
import { SecretOnce } from "./SecretOnce.tsx";

export function Copy({ text }: { text: string }) {
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

function SnippetBlock({ s }: { s: Snippet }) {
  return (
    <div className="snippet">
      <div className="snippet-head"><strong>{s.title}</strong><Copy text={s.text} /></div>
      <div className="snippet-scroll"><pre className="snippet-body">{s.text}</pre></div>
    </div>
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
  const all = snippets(base);
  return (
    <>
      <section className="card">
        <h3>Connect an agent</h3>
        <p className="small">Paste the MCP URL into {OAUTH_CLIENTS} and sign in when the browser opens. No token to copy.</p>
        {all.filter((s) => !s.token).map((s) => <SnippetBlock key={s.id} s={s} />)}
        <div className="edit-actions">
          <a className="button ghost" href={llmsTxt(base)} target="_blank" rel="noreferrer">Read llms.txt</a>
        </div>
      </section>
      <section className="card">
        <h3>Connect with a token</h3>
        <p className="muted small">{TOKEN_FALLBACK} <a href="#/people">Add the agent in People</a>: it gets its own token and its setup filled in, with one-click links for Cursor and VS Code. Here, replace <code className="mono-id">{TOKEN_PLACEHOLDER}</code> with that token.</p>
        {all.filter((s) => s.token).map((s) => <SnippetBlock key={s.id} s={s} />)}
        <div className="edit-actions">
          <a className="button steel" href={cursorDeeplink(base)}>Add to Cursor</a>
        </div>
      </section>
      {signedIn ? (
        <section className="card">
          <h3>First admin token</h3>
          <p className="muted small">You do not need this to connect an agent: add agents in People. If you created this workspace and want an admin token for yourself (to script against the API), mint it here; it is shown once. Never give the admin token to an agent.</p>
          <button type="button" className="button steel" disabled={busy} onClick={() => void mint()}>{busy ? "Minting…" : "Mint an admin token"}</button>
          {note ? <p className="err small" role="alert">{note}</p> : null}
          {secret ? <SecretOnce title="Admin token" secret={secret} onDone={() => setSecret(null)} /> : null}
        </section>
      ) : null}
    </>
  );
}
