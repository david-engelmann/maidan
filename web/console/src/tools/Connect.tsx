// Connect an agent to this workspace over MCP, and mint the first admin
// token for this session. Signing in with OAuth comes first; a token is the
// fallback. Naming an agent here mints its worker token once. Snippets that
// are only a reference carry a placeholder, never a token.
import { useMemo, useState } from "react";
import { OAUTH_CLIENTS, TOKEN_FALLBACK, TOKEN_PLACEHOLDER, cursorOAuthDeeplink, llmsTxt, snippets, vscodeOAuthDeeplink, type Snippet } from "../core/connect.ts";
import { PeopleApi } from "../core/people.ts";
import type { ToolsApi } from "../core/tools.ts";
import { useSession } from "../hooks/useSession.ts";
import { AddAgent } from "../people/AddAgent.tsx";
import { Copy, SecretOnce } from "./SecretOnce.tsx";

// First-run still imports Copy from this module.
export { Copy } from "./SecretOnce.tsx";

function SnippetBlock({ s }: { s: Snippet }) {
  return (
    <div className="snippet">
      <div className="snippet-head"><strong>{s.title}</strong><Copy text={s.text} /></div>
      {/* A long line scrolls sideways on a phone; the region has to take keyboard focus. */}
      <div className="snippet-scroll"><pre className="snippet-body" tabIndex={0}>{s.text}</pre></div>
    </div>
  );
}

// A private window can refuse storage. No channel is the same as none picked.
function storedChannel(): string | null {
  try {
    return localStorage.getItem("maidan_console_channel");
  } catch {
    return null;
  }
}

export function Connect({ api, base, signedIn }: { api: ToolsApi; base: string; signedIn: boolean }) {
  const { session, store } = useSession();
  const peopleApi = useMemo(() => {
    const creds = () => store.getSnapshot().credentials;
    const mode = () => store.getSnapshot().mode;
    const f = (url: string, init?: RequestInit) => fetch(url, init);
    return new PeopleApi(f, creds, mode);
  }, [store]);
  const workspaceId = session.status === "signed-in" ? session.workspaceId : null;
  const channelId = storedChannel();
  const room = `${signedIn ? "in" : "out"}:${workspaceId ?? ""}`;
  const [secret, setSecret] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [seenRoom, setSeenRoom] = useState(room);
  if (seenRoom !== room) {
    setSeenRoom(room);
    setSecret(null);
    setNote(null);
  }
  const [busy, setBusy] = useState(false);
  const [showTemplates, setShowTemplates] = useState(false);
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
  const tokenSnippets = all.filter((s) => s.token);
  const canMint = Boolean(signedIn && workspaceId);
  return (
    <>
      <section className="card">
        <h3>Connect an agent</h3>
        <p className="small">Paste the MCP URL into {OAUTH_CLIENTS} and sign in when the browser opens. No token to copy.</p>
        {all.filter((s) => !s.token).map((s) => <SnippetBlock key={s.id} s={s} />)}
        {/* edit-actions stays one line; row-form lets the three links wrap on a phone. */}
        <div className="edit-actions row-form">
          <a className="button steel" href={cursorOAuthDeeplink(base)}>Add to Cursor</a>
          <a className="button ghost" href={vscodeOAuthDeeplink(base)}>Add to VS Code</a>
          <a className="button ghost" href={llmsTxt(base)} target="_blank" rel="noreferrer">Read llms.txt</a>
        </div>
      </section>
      <section className="card">
        <h3>Connect with a token</h3>
        {canMint && workspaceId ? (
          <>
            <p className="muted small">{TOKEN_FALLBACK} Name an agent here to mint a scoped <code className="mono-id">maidan.agent.worker</code> token once and fill in its client setup, with one-click install links for Cursor and VS Code.</p>
            <AddAgent key={workspaceId ?? ""} api={peopleApi} workspace={workspaceId} base={base} channelId={channelId} />
            <div className="row-form">
              <button type="button" className="button ghost" aria-expanded={showTemplates} aria-controls="connect-token-templates" onClick={() => setShowTemplates((open) => !open)}>
                Already have a token?
              </button>
            </div>
            <div id="connect-token-templates">
              {showTemplates ? (
                <>
                  <p className="muted small">Replace <code className="mono-id">{TOKEN_PLACEHOLDER}</code> with your token.</p>
                  {tokenSnippets.map((s) => <SnippetBlock key={s.id} s={s} />)}
                </>
              ) : null}
            </div>
          </>
        ) : (
          <>
            <p className="muted small">{TOKEN_FALLBACK} Sign in to add an agent and mint its worker token in one click.</p>
            <p className="muted small">Replace <code className="mono-id">{TOKEN_PLACEHOLDER}</code> with your token.</p>
            {tokenSnippets.map((s) => <SnippetBlock key={s.id} s={s} />)}
          </>
        )}
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
