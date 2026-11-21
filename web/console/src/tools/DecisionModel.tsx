// A workspace's own keys for decision models, for advice on hand-offs. Each
// provider has its own key, typed here once, checked by that provider, stored
// encrypted, and never read back: the panel only ever knows whether one is set.
import { type FormEvent, useMemo, useState } from "react";
import { AdviceApi, KEY_NAME, type AdvisorProvider, type AdvisorStatus } from "../core/advice.ts";
import { say, sayError } from "../core/notices.ts";
import { useSession } from "../hooks/useSession.ts";
import { useLoad } from "../hooks/useTools.ts";

/** One line on where advice from a provider stands for this workspace. */
export function providerLine(p: Pick<AdvisorProvider, "key_source">): string {
  if (p.key_source === "workspace") return "On, with this workspace’s own key.";
  if (p.key_source === "instance") return "On, with this server’s key. Add your own to use your own account instead.";
  return "Off for this workspace until it has a key.";
}

/** A server from before it named its providers offers the one it describes. */
function providersOf(status: AdvisorStatus): AdvisorProvider[] {
  if (status.providers?.length) return status.providers;
  return [{ id: "typesafe", name: "TypeSafe Jev", model: status.model ?? "", host: status.host ?? "", key_source: status.key_source }];
}

export function DecisionModel({ ws }: { ws: string }) {
  const { store } = useSession();
  const api = useMemo(
    () => new AdviceApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode),
    [store],
  );
  const status = useLoad(`advisor:${ws}`, () => api.status(ws));

  const retry = (
    <button type="button" className="link-btn" onClick={() => void status.reload()}>Try again</button>
  );
  if (status.value === null) {
    // Still reading. A read that failed says so: a hidden panel would leave
    // nothing to retry.
    if (status.status !== "error") return null;
    return (
      <section className="card tool-panel">
        <div className="panel-head">
          <h2>Decision models</h2>
        </div>
        <p className="small" role="alert">Could not read how advice is set up here: {status.error} {retry}</p>
      </section>
    );
  }
  // A server that does not offer advice has nothing to set up.
  if (!status.value.enabled) return null;
  const providers = providersOf(status.value);
  const on = providers.some((p) => p.key_source);

  return (
    <section className="card tool-panel">
      <div className="panel-head">
        <h2>Decision models</h2>
        <span className="state-chip">{on ? "On" : "Off"}</span>
      </div>
      <p className="muted small">
        A decision model reads a task’s hand-off and answers ten yes/no questions about it. The answers show on the task as advice for whoever reviews it.
        {providers.length > 1 ? " Add a key for any of these; with more than one, a reviewer can switch between them on the task." : ""}
      </p>
      <p className="small">Only this workspace’s admin can set or remove a key.</p>
      {status.error ? <p className="small" role="alert">This may be out of date. The last read failed: {status.error} {retry}</p> : null}
      <ul className="muted small advisor-facts">
        <li><b>When.</b> Advice is asked for when someone opens a task that is in review, or an agent asks for it. Until then nothing is sent.</li>
        <li><b>What is sent.</b> To the provider asked, and no other: the task’s title, description and acceptance, its result, and the messages of the agent that did the work. What the provider keeps is under its own terms.</li>
        <li><b>The key.</b> Saving sends it to its provider once, to check that it works. It is then stored encrypted here and never shown again.</li>
      </ul>
      <ul className="advisor-providers">
        {providers.map((p) => (
          <Provider key={p.id} ws={ws} api={api} provider={p} reload={() => status.reload()} />
        ))}
      </ul>
    </section>
  );
}

function Provider({ ws, api, provider, reload }: { ws: string; api: AdviceApi; provider: AdvisorProvider; reload: () => Promise<unknown> }) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<"set" | "clear" | null>(null);
  const [confirming, setConfirming] = useState(false);
  const own = provider.key_source === "workspace";

  const run = async (kind: "set" | "clear", work: () => Promise<unknown>, done: string, fallback: string) => {
    if (busy) return;
    setBusy(kind);
    try {
      await work();
      say(done);
      await reload();
    } catch (e) {
      sayError(e, fallback);
    } finally {
      setBusy(null);
    }
  };

  return (
    <li className="advisor-provider" data-provider={provider.id}>
      <h3>
        {provider.name}
        <span className={`state-chip${provider.key_source ? " active" : ""}`}>{provider.key_source ? "On" : "Off"}</span>
      </h3>
      <p className="small">{providerLine(provider)}</p>
      <p className="muted small">Sent to {provider.host}{provider.model ? `, model ${provider.model}` : ""}.</p>
      <form
        className="row-form"
        onSubmit={(e: FormEvent) => {
          e.preventDefault();
          void run("set", async () => { await api.setKey(ws, key.trim(), provider.id); setKey(""); }, `Key saved. ${provider.name} is on for this workspace.`, "Could not save the key");
        }}
      >
        <label className="field grow">
          {own ? `Replace the ${KEY_NAME[provider.id]}` : `Your ${KEY_NAME[provider.id]}`}
          <input type="password" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" spellCheck={false} autoCapitalize="none" placeholder="Paste the key" />
        </label>
        <button className="button steel" disabled={busy !== null || !key.trim()} aria-label={`Save the ${KEY_NAME[provider.id]}`}>{busy === "set" ? "Checking…" : "Save"}</button>
      </form>
      {own ? (
        confirming ? (
          <p className="small">
            Remove this workspace’s {KEY_NAME[provider.id]}?{" "}
            <button type="button" className="chip-btn danger" disabled={busy !== null} onClick={() => void run("clear", async () => { await api.clearKey(ws, provider.id); setConfirming(false); }, "Key removed", "Could not remove the key")}>Remove</button>{" "}
            <button type="button" className="chip-btn" onClick={() => setConfirming(false)}>Keep</button>
          </p>
        ) : (
          <p className="small"><button type="button" className="link-btn" onClick={() => setConfirming(true)} aria-label={`Remove the ${KEY_NAME[provider.id]}…`}>Remove the key…</button></p>
        )
      ) : null}
    </li>
  );
}
