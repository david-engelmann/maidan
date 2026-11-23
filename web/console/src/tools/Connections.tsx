// A workspace's own GitHub, Slack and Linear. Each row says whose account the
// surface runs on, takes a token once (checked by the provider, stored
// encrypted, never shown again) and, for GitHub and Slack, shows the URL the
// provider posts to. Reads and writes are admin-only on the server too, so
// anyone else gets a sentence, not a refused request.
import { type FormEvent, type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import {
  CONNECTIONS_HELP,
  CONNECTIONS_NEED,
  ConnectionsApi,
  connectRefusal,
  inboundLine,
  scrub,
  stateLine,
  stateWord,
  surfaceInfo,
  surfaceName,
  type Connection,
  type ConnectionSurface,
} from "../core/connections.ts";
import { asksForAllowlist } from "../core/deliveries.ts";
import { say } from "../core/notices.ts";
import { useSession } from "../hooks/useSession.ts";
import { useLoad } from "../hooks/useTools.ts";

export function Connections({ ws }: { ws: string | null }) {
  const { session, store } = useSession();
  const api = useMemo(
    () => new ConnectionsApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode),
    [store],
  );
  // The same gate as the egress allowlist: a token is exactly its grant, so
  // one without token:admin is told so with no request. A person's own
  // sign-in carries none of its own, so only the server can answer for it.
  const tokenBacked = session.status === "signed-in" && (Boolean(session.credentials.pastedToken.trim()) || session.credentials.tokenSession);
  const deciding = tokenBacked && session.grant === null;
  const asks = ws && !deciding && asksForAllowlist(tokenBacked, session.grant) ? ws : null;
  const list = useLoad(asks ? `connections:${asks}` : null, () => api.list(asks ?? ""));
  const title = "GitHub, Slack and Linear";
  const shell = (body: ReactNode, action?: ReactNode) => (
    <section className="card tool-panel wide">
      <div className="panel-head">
        <h2>{title}</h2>
        {action}
      </div>
      {body}
    </section>
  );
  if (!ws) return shell(<p className="muted">Sign in as a member to see this.</p>);
  if (deciding) return shell(<p className="muted">Loading…</p>);
  if (!asks) return shell(<p className="muted">{CONNECTIONS_NEED}</p>);
  if (list.status === "loading") return shell(<p className="muted">Loading…</p>);
  if (list.value === null) {
    return shell(
      <div className="load-failed" role="alert">
        <p className="err">{list.error}</p>
        <button type="button" className="btn" onClick={() => void list.reload()}>Try again</button>
      </div>,
    );
  }
  if (!list.value.allowed) return shell(<p className="muted">{CONNECTIONS_NEED}</p>);
  const rows = list.value.value;
  return shell(
    <>
      <p className="muted small">{CONNECTIONS_HELP}</p>
      {list.error ? <p className="err small" role="alert">This may be out of date. The last read failed. {list.error}</p> : null}
      {rows.length ? (
        <ul className="connection-list" aria-label="Connections">
          {rows.map((row) => (
            <ConnectionRow key={row.surface} ws={ws} api={api} row={row} reload={() => list.reload()} />
          ))}
        </ul>
      ) : (
        <p className="muted">The server listed no surfaces to connect.</p>
      )}
    </>,
  );
}

function ConnectionRow({ ws, api, row, reload }: { ws: string; api: ConnectionsApi; row: Connection; reload: () => Promise<unknown> }) {
  const surface: ConnectionSurface = row.surface;
  const info = surfaceInfo(surface);
  const name = surfaceName(surface);
  const own = row.source === "workspace";
  const [token, setToken] = useState("");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState<"connect" | "remove" | null>(null);
  const [why, setWhy] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const item = useRef<HTMLLIElement>(null);
  const tokenInput = useRef<HTMLInputElement>(null);
  const removeBtn = useRef<HTMLButtonElement>(null);
  const keepBtn = useRef<HTMLButtonElement>(null);
  const confirmed = useRef(false);
  const id = `connection-${surface}`;
  const field = info?.secret?.field;

  // Asking for a confirmation puts focus on the safe choice, and cancelling
  // puts it back where the person was.
  useEffect(() => {
    if (confirming) keepBtn.current?.focus();
    else if (confirmed.current) {
      confirmed.current = false;
      removeBtn.current?.focus();
    }
  }, [confirming]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy) return;
    const input = { token, ...(field ? { [field]: secret } : {}) };
    const typed = [token, secret];
    const local = connectRefusal(surface, input, own);
    if (local) {
      setWhy(local);
      tokenInput.current?.focus();
      return;
    }
    setBusy("connect");
    setWhy(null);
    // What was typed leaves the page whatever the answer, so a secret never
    // sits in a field after it has been sent.
    setToken("");
    setSecret("");
    try {
      const saved = await api.connect(ws, surface, input);
      say(saved.account ? `${name} connected as ${saved.account}` : `${name} connected`);
      await reload();
      item.current?.focus();
    } catch (err) {
      setWhy(err instanceof Error && err.message ? scrub(err.message, ...typed) : `Could not connect ${name}`);
      tokenInput.current?.focus();
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (busy) return;
    setBusy("remove");
    try {
      await api.remove(ws, surface);
      say(`${name} disconnected`);
      confirmed.current = false;
      setConfirming(false);
      await reload();
      item.current?.focus();
    } catch (err) {
      setWhy(err instanceof Error && err.message ? err.message : `Could not remove the ${name} connection`);
      confirmed.current = true;
      setConfirming(false);
    } finally {
      setBusy(null);
    }
  };

  const tokenLabel = own ? `Replace the ${info?.token ?? "token"}` : `${info?.token ?? "Token"}`;
  const sendable = Boolean(token.trim()) || (own && Boolean(secret.trim()));
  return (
    <li ref={item} tabIndex={-1} className="tool-row connection-row" data-surface={surface} data-source={row.source ?? "none"}>
      <div className="grow">
        <div className="token-head">
          <h3 id={`${id}-name`}>{name}</h3>
          <span className={`state-chip${own ? " active" : ""}`}>{stateWord(row.source)}</span>
        </div>
        <p className="small">{stateLine(row)}</p>
        {row.inbound && info ? <Inbound surface={surface} inbound={row.inbound} paste={info.paste} id={id} /> : null}
        <form className="row-form" aria-label={`${name} connection`} onSubmit={(e) => void submit(e)}>
          <div className="field grow">
            <label htmlFor={`${id}-token`}>{tokenLabel}</label>
            <input
              id={`${id}-token`}
              ref={tokenInput}
              type="password"
              value={token}
              onChange={(e) => { setToken(e.target.value); setWhy(null); }}
              autoComplete="off"
              spellCheck={false}
              autoCapitalize="none"
              placeholder="Paste it here"
              aria-invalid={why !== null}
              aria-describedby={why ? `${id}-why` : undefined}
            />
          </div>
          {info?.secret ? (
            <div className="field grow">
              <label htmlFor={`${id}-secret`}>{`${info.secret.label} (optional)`}</label>
              <input
                id={`${id}-secret`}
                type="password"
                value={secret}
                onChange={(e) => { setSecret(e.target.value); setWhy(null); }}
                autoComplete="off"
                spellCheck={false}
                autoCapitalize="none"
                placeholder={own ? "Leave empty to keep the current one" : "Paste it here"}
                aria-describedby={`${id}-secret-hint`}
              />
              <span className="muted small" id={`${id}-secret-hint`}>{info.secret.hint}</span>
            </div>
          ) : null}
          <button className="button steel" disabled={busy !== null || !sendable} aria-label={own ? `Replace the ${name} connection` : `Connect ${name}`}>
            {busy === "connect" ? "Checking…" : own ? "Replace" : "Connect"}
          </button>
        </form>
        {why ? <p className="err small" id={`${id}-why`} role="alert">{why}</p> : null}
      </div>
      {own ? (
        <div className="row-actions">
          {confirming ? (
            <>
              <button type="button" className="chip-btn danger confirm" disabled={busy !== null} onClick={() => void remove()}>
                {busy === "remove" ? "Removing…" : "Remove"}
              </button>
              <button ref={keepBtn} type="button" className="chip-btn" onClick={() => { confirmed.current = true; setConfirming(false); }}>Keep</button>
            </>
          ) : (
            <button ref={removeBtn} type="button" className="chip-btn" onClick={() => setConfirming(true)} aria-label={`Remove the ${name} connection…`}>Remove…</button>
          )}
        </div>
      ) : null}
    </li>
  );
}

/** The URL a provider posts to, to copy, with where it goes and whether it has a secret to sign with. */
function Inbound({ surface, inbound, paste, id }: { surface: ConnectionSurface; inbound: NonNullable<Connection["inbound"]>; paste: string; id: string }) {
  const [copied, setCopied] = useState<"no" | "yes" | "failed">("no");
  useEffect(() => {
    if (copied !== "yes") return;
    const timer = setTimeout(() => setCopied("no"), 2500);
    return () => clearTimeout(timer);
  }, [copied]);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(inbound.url);
      setCopied("yes");
    } catch {
      setCopied("failed");
    }
  };
  return (
    <div className="connection-inbound">
      <p className="small"><b>Inbound URL</b></p>
      <div className="connection-url-row">
        <code className="connection-url break" id={`${id}-url`}>{inbound.url}</code>
        <button type="button" className="chip-btn" onClick={() => void copy()} aria-label={`Copy the ${surfaceName(surface)} inbound URL`}>{copied === "yes" ? "Copied" : "Copy"}</button>
      </div>
      {copied === "yes" ? <span className="visually-hidden" role="status" aria-live="polite">Copied</span> : null}
      {copied === "failed" ? <p className="err small" role="alert">This browser would not copy. Select the URL and copy it by hand.</p> : null}
      <p className="muted small">{paste}</p>
      <p className="muted small">{inboundLine(surface, inbound)}</p>
    </div>
  );
}
