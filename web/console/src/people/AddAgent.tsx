// "Add an agent": a name, a client, then the agent's own token shown once with
// that client's setup filled in. Used by the board's first-run card and by
// People. The token lives only in this component's state and goes when the
// person says they have it.
import { useState } from "react";
import { CLIENTS, HANDLE_MAX, SHOWN_ONCE, clientSetup, handleFrom, type ClientId } from "../core/addAgent.ts";
import type { PeopleApi } from "../core/people.ts";
import { Copy } from "../tools/Connect.tsx";
import { SecretOnce } from "../tools/SecretOnce.tsx";

export function AddAgent({ api, workspace, base, channelId, onAdded }: { api: PeopleApi; workspace: string; base: string; channelId: string | null; onAdded?: () => void }) {
  const [name, setName] = useState("");
  const [client, setClient] = useState<ClientId>("claude");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [added, setAdded] = useState<{ handle: string; secret: string } | null>(null);
  const handle = handleFrom(name);

  const add = async () => {
    if (busy) return;
    setBusy(true);
    setNote(null);
    try {
      const out = await api.addAgent(workspace, name);
      setAdded({ handle: out.handle, secret: out.secret });
      setName("");
      onAdded?.();
    } catch (e) {
      setNote(e instanceof Error ? e.message : "Could not add the agent");
    } finally {
      setBusy(false);
    }
  };

  const picker = (
    <div className="view-pick client-pick" role="group" aria-label="Client">
      {CLIENTS.map((c) => (
        <button key={c.id} type="button" className="chip-btn" aria-pressed={client === c.id} onClick={() => setClient(c.id)}>
          {c.name}
        </button>
      ))}
    </div>
  );

  if (added) {
    return (
      <div className="add-agent">
        <SecretOnce title={`Token for @${added.handle}`} secret={added.secret} onDone={() => setAdded(null)} />
        <p className="small">Set up {CLIENTS.find((c) => c.id === client)?.name ?? "your client"}. {SHOWN_ONCE}</p>
        {picker}
        {clientSetup(client, base, added.secret, channelId).map((s) => (
          <div key={s.title} className="snippet">
            <div className="snippet-head">
              {s.href ? <a className="button steel" href={s.href}>{s.title}</a> : <span className="small">{s.title}</span>}
              <Copy text={s.text} />
            </div>
            <div className="snippet-scroll"><pre className="snippet-body">{s.text}</pre></div>
          </div>
        ))}
      </div>
    );
  }

  return (
    <form
      className="add-agent"
      onSubmit={(e) => {
        e.preventDefault();
        void add();
      }}
    >
      <div className="row-form">
        <label className="field">
          Agent name
          <input value={name} maxLength={100} placeholder="Claude" autoComplete="off" onChange={(e) => setName(e.target.value)} />
        </label>
        <button type="submit" className="button ink" disabled={busy || !handle}>{busy ? "Adding…" : "Add agent"}</button>
      </div>
      <p className="muted small">{handle ? `It joins as @${handle}, with a worker token: it can take and finish tasks, never mint tokens.` : `Letters, digits, '-', '_' and '.' make its handle, up to ${HANDLE_MAX}.`}</p>
      {picker}
      {note ? <p className="err small" role="alert">{note}</p> : null}
    </form>
  );
}
