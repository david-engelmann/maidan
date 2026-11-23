// A workspace's outbound webhooks: the URLs Maidan posts matching events to.
// Adding one answers with its signing secret once, shown with SecretOnce and
// held nowhere else. The routes take a token, so a browser sign-in is told how
// to get one instead of being sent to a route that is not there.
import { type FormEvent, type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { connectedOn } from "../core/connections.ts";
import { hasToken } from "../core/api.ts";
import { say } from "../core/notices.ts";
import { EVENT_KINDS } from "../core/tools.ts";
import {
  WEBHOOKS_NEED_TOKEN,
  WEBHOOKS_NO_READ,
  WEBHOOK_SECRET_NOTE,
  WEBHOOK_URL_RULE,
  WebhooksApi,
  eventWord,
  webhookName,
  type MintedWebhook,
  type Webhook,
} from "../core/webhooks.ts";
import { useSession } from "../hooks/useSession.ts";
import { useLoad } from "../hooks/useTools.ts";
import { SecretOnce } from "./SecretOnce.tsx";

export function Webhooks({ ws }: { ws: string | null }) {
  const { session, store } = useSession();
  const api = useMemo(
    () => new WebhooksApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode),
    [store],
  );
  const canCall = session.status === "signed-in" && hasToken(session.credentials);
  const list = useLoad(ws && canCall ? `webhooks:${ws}` : null, () => api.list(ws ?? ""));
  const [secret, setSecret] = useState<{ name: string; value: string } | null>(null);
  const [url, setUrl] = useState("");
  const [label, setLabel] = useState("");
  const [kinds, setKinds] = useState<string[]>([]);
  const [why, setWhy] = useState<string | null>(null);
  const [dropWhy, setDropWhy] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [dropping, setDropping] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<string | null>(null);
  const urlInput = useRef<HTMLInputElement>(null);
  const panel = useRef<HTMLElement>(null);
  const keepBtn = useRef<HTMLButtonElement>(null);
  const backTo = useRef<string | null>(null);
  const shell = (body: ReactNode) => (
    <section ref={panel} tabIndex={-1} aria-labelledby="webhooks-title" className="card tool-panel wide">
      <div className="panel-head">
        <h2 id="webhooks-title">Webhooks</h2>
      </div>
      {body}
    </section>
  );

  // Asking to revoke puts focus on the safe choice. Cancelling puts it back on
  // the button that asked.
  useEffect(() => {
    if (dropping) keepBtn.current?.focus();
    else if (backTo.current) {
      document.getElementById(backTo.current)?.focus();
      backTo.current = null;
    }
  }, [dropping]);

  if (!ws) return shell(<p className="muted">Sign in as a member to see this.</p>);
  if (!canCall) return shell(<p className="muted">{WEBHOOKS_NEED_TOKEN}</p>);
  if (list.status === "loading") return shell(<p className="muted">Loading…</p>);
  if (list.value === null) {
    return shell(
      <div className="load-failed" role="alert">
        <p className="err">{list.error}</p>
        <button type="button" className="btn" onClick={() => void list.reload()}>Try again</button>
      </div>,
    );
  }
  if (!list.value.allowed) return shell(<p className="muted">{WEBHOOKS_NO_READ}</p>);
  const rows = list.value.value;

  const toggle = (kind: string) => {
    setKinds((now) => (now.includes(kind) ? now.filter((k) => k !== kind) : [...now, kind]));
    setWhy(null);
  };
  const add = async (e: FormEvent) => {
    e.preventDefault();
    if (adding) return;
    setAdding(true);
    setWhy(null);
    try {
      const made: MintedWebhook = await api.create(ws, { url, label, event_kinds: EVENT_KINDS.filter((k) => kinds.includes(k)) });
      setSecret({ name: webhookName(made.webhook), value: made.secret });
      setUrl("");
      setLabel("");
      setKinds([]);
      say(`Webhook added for ${webhookName(made.webhook)}`);
      await list.reload();
    } catch (err) {
      setWhy(err instanceof Error && err.message ? err.message : "Could not add the webhook");
      urlInput.current?.focus();
    } finally {
      setAdding(false);
    }
  };
  const revoke = async (w: Webhook) => {
    if (revoking) return;
    setRevoking(w.id);
    setDropWhy(null);
    try {
      await api.revoke(ws, w.id);
      say(`Webhook revoked for ${webhookName(w)}`);
      setDropping(null);
      await list.reload();
      panel.current?.focus();
    } catch (err) {
      setDropWhy(err instanceof Error && err.message ? err.message : "Could not revoke the webhook");
      backTo.current = `webhook-revoke-${w.id}`;
      setDropping(null);
    } finally {
      setRevoking(null);
    }
  };

  return shell(
    <>
      <p className="muted small">A webhook is a URL Maidan posts matching events to as they happen in this workspace. It receives every matching event, private channels and direct messages included.</p>
      {list.error ? <p className="err small" role="alert">This may be out of date. The last read failed. {list.error}</p> : null}
      {secret ? (
        <SecretOnce
          title={`Signing secret for ${secret.name}`}
          secret={secret.value}
          note={WEBHOOK_SECRET_NOTE}
          onDone={() => {
            setSecret(null);
            urlInput.current?.focus();
          }}
        />
      ) : null}
      {dropWhy ? <p className="err small" role="alert">{dropWhy}</p> : null}
      {rows.length ? (
        <ul className="tool-list" aria-label="Webhooks">
          {rows.map((w) => (
            <li key={w.id} tabIndex={-1} className="tool-row webhook-row" data-webhook={w.id}>
              <div className="grow">
                <div className="token-head"><b className="break">{webhookName(w)}</b></div>
                {w.label?.trim() ? <p className="small break">{w.url}</p> : null}
                <ul className="webhook-kinds" aria-label={`Events sent to ${webhookName(w)}`}>
                  {w.event_kinds.map((k) => <li key={k} className="state-chip">{eventWord(k)}</li>)}
                </ul>
                <p className="muted small">Added <time dateTime={w.created_at} title={new Date(w.created_at).toLocaleString()}>{connectedOn(w.created_at) ?? w.created_at}</time></p>
              </div>
              <div className="row-actions">
                {dropping === w.id ? (
                  <>
                    <button type="button" className="chip-btn danger confirm" disabled={revoking !== null} onClick={() => void revoke(w)}>{revoking === w.id ? "Revoking…" : "Revoke"}</button>
                    <button ref={keepBtn} type="button" className="chip-btn" onClick={() => { backTo.current = `webhook-revoke-${w.id}`; setDropping(null); }}>Keep</button>
                  </>
                ) : (
                  <button id={`webhook-revoke-${w.id}`} type="button" className="chip-btn" onClick={() => setDropping(w.id)} aria-label={`Revoke the webhook for ${webhookName(w)}…`}>Revoke…</button>
                )}
              </div>
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted">No webhooks yet. Add one to have matching events posted to a URL you run.</p>
      )}
      <form className="stack-form" aria-label="Add a webhook" onSubmit={(e) => void add(e)}>
        <div className="field">
          <label htmlFor="webhook-url">URL</label>
          <input
            id="webhook-url"
            ref={urlInput}
            value={url}
            onChange={(e) => { setUrl(e.target.value); setWhy(null); }}
            placeholder="https://example.com/maidan"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            aria-invalid={why !== null}
            aria-describedby={`webhook-url-rule${why ? " webhook-why" : ""}`}
          />
          <span className="muted small" id="webhook-url-rule">{WEBHOOK_URL_RULE}</span>
        </div>
        <div className="field">
          <label htmlFor="webhook-label">Label (optional)</label>
          <input id="webhook-label" value={label} onChange={(e) => setLabel(e.target.value)} autoComplete="off" />
        </div>
        <fieldset className="webhook-events" aria-describedby="webhook-events-hint">
          <legend>Events</legend>
          <div className="webhook-event-list">
            {EVENT_KINDS.map((k) => (
              <label key={k} className="check">
                <input type="checkbox" checked={kinds.includes(k)} onChange={() => toggle(k)} />
                {eventWord(k)}
              </label>
            ))}
          </div>
          <span className="muted small" id="webhook-events-hint">Choose at least one.</span>
        </fieldset>
        <div>
          <button className="button steel" disabled={adding || !url.trim() || kinds.length === 0}>{adding ? "Adding…" : "Add webhook"}</button>
        </div>
      </form>
      {why ? <p className="err small" id="webhook-why" role="alert">{why}</p> : null}
    </>,
  );
}
