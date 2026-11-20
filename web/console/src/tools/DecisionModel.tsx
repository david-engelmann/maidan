// The workspace's own TypeSafe key, for advice on hand-offs. The key is
// typed here once, checked by the provider, stored encrypted, and never read
// back: the panel only ever knows whether one is set.
import { type FormEvent, useMemo, useState } from "react";
import { AdviceApi, type AdvisorStatus } from "../core/advice.ts";
import { say, sayError } from "../core/notices.ts";
import { useSession } from "../hooks/useSession.ts";
import { useLoad } from "../hooks/useTools.ts";

/** One line on where advice stands for this workspace. */
export function advisorLine(status: AdvisorStatus): string {
  if (status.key_source === "workspace") return "On, with this workspace’s own key.";
  if (status.key_source === "instance") return "On, with this server’s key. Add your own to use your TypeSafe account instead.";
  return "Off for this workspace until it has a key.";
}

export function DecisionModel({ ws }: { ws: string }) {
  const { store } = useSession();
  const api = useMemo(
    () => new AdviceApi((url, init) => fetch(url, init), () => store.getSnapshot().credentials, () => store.getSnapshot().mode),
    [store],
  );
  const status = useLoad(`advisor:${ws}`, () => api.status(ws));
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<"set" | "clear" | null>(null);
  const [confirming, setConfirming] = useState(false);

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
          <h2>Decision model</h2>
        </div>
        <p className="small" role="alert">Could not read how advice is set up here: {status.error} {retry}</p>
      </section>
    );
  }
  // A server that does not offer advice has nothing to set up.
  if (!status.value.enabled) return null;
  const current = status.value;

  const run = async (kind: "set" | "clear", work: () => Promise<unknown>, done: string, fallback: string) => {
    if (busy) return;
    setBusy(kind);
    try {
      await work();
      say(done);
      await status.reload();
    } catch (e) {
      sayError(e, fallback);
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className="card tool-panel">
      <div className="panel-head">
        <h2>Decision model</h2>
        <span className="state-chip">{current.key_source ? "On" : "Off"}</span>
      </div>
      <p className="muted small">
        {current.model?.startsWith("jev") ? "TypeSafe’s Jev" : "A decision model"} reads a task’s hand-off and answers ten yes/no questions about it. The answers show on the task as advice for whoever reviews it.
      </p>
      <p className="small">{advisorLine(current)} Only this workspace’s admin can set or remove the key.</p>
      {status.error ? <p className="small" role="alert">This may be out of date. The last read failed: {status.error} {retry}</p> : null}
      <ul className="muted small advisor-facts">
        <li><b>When.</b> Advice is asked for when someone opens a task that is in review, or an agent asks for it. Until then nothing is sent.</li>
        <li><b>What is sent.</b> To {current.host ?? "the provider"}: the task’s title, description and acceptance, its result, and the messages of the agent that did the work. What TypeSafe keeps is under its own terms.</li>
        <li><b>The key.</b> Saving sends it to TypeSafe once, to check that it works. It is then stored encrypted here and never shown again.</li>
      </ul>
      <form
        className="row-form"
        onSubmit={(e: FormEvent) => {
          e.preventDefault();
          void run("set", async () => { await api.setKey(ws, key.trim()); setKey(""); }, "Key saved. Advice is on for this workspace.", "Could not save the key");
        }}
      >
        <label className="field grow">
          {current.key_source === "workspace" ? "Replace the key with another" : "Your TypeSafe API key"}
          <input type="password" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" spellCheck={false} autoCapitalize="none" placeholder="Paste the key" />
        </label>
        <button className="button steel" disabled={busy !== null || !key.trim()}>{busy === "set" ? "Checking…" : "Save"}</button>
      </form>
      {current.key_source === "workspace" ? (
        confirming ? (
          <p className="small">
            Remove this workspace’s key?{" "}
            <button type="button" className="chip-btn danger" disabled={busy !== null} onClick={() => void run("clear", async () => { await api.clearKey(ws); setConfirming(false); }, "Key removed", "Could not remove the key")}>Remove</button>{" "}
            <button type="button" className="chip-btn" onClick={() => setConfirming(false)}>Keep</button>
          </p>
        ) : (
          <p className="small"><button type="button" className="link-btn" onClick={() => setConfirming(true)}>Remove the key…</button></p>
        )
      ) : null}
    </section>
  );
}
