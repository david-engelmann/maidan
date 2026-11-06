// Files: the content-addressed artifact store. Look one up by its sha256,
// upload a file to get one, and link files to a task as its evidence.
import { type FormEvent, useEffect, useMemo, useState } from "react";
import { BoardApi, type Thread } from "../core/board.ts";
import { say, sayError } from "../core/notices.ts";
import { formatBytes, isSha, previewable, type ArtifactMeta, type ToolsApi } from "../core/tools.ts";
import { useSession } from "../hooks/useSession.ts";
import { useLoad, useToolsApi } from "../hooks/useTools.ts";
import { Library } from "./Library.tsx";

/** A handful of reads at a time, matching the file library. */
async function mapLimited<T, R>(items: T[], n: number, f: (t: T) => Promise<R>): Promise<R[]> {
  const out: R[] = new Array(items.length);
  let i = 0;
  await Promise.all(
    Array.from({ length: Math.min(n, items.length) }, async () => {
      while (i < items.length) {
        const k = i++;
        out[k] = await f(items[k]);
      }
    }),
  );
  return out;
}

export type TaskChoice = { id: string; channel: string; title: string };

/** `#general — Deploy v9`, the line in the task picker. */
export function taskOptionLabel(channel: string, title: string): string {
  const name = channel.trim().replace(/^#/, "") || "channel";
  return `#${name} — ${title.trim() || "Untitled task"}`;
}

export function Artifacts() {
  const api = useToolsApi();
  const { session, store } = useSession();
  const board = useMemo(() => new BoardApi((u, i) => fetch(u, i), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const ws = session.status === "signed-in" ? session.workspaceId : null;
  const tasks = useLoad(ws, async () => {
    const channels = await board.channels(ws ?? "");
    const lists = await mapLimited(channels, 4, async (c) => {
      const found = await board.threads(c.id).catch(() => [] as Thread[]);
      return found.map((t) => ({ id: t.id, channel: c.name, title: t.title?.trim() || "Untitled task" }));
    });
    return lists.flat().sort((a, b) => a.channel.localeCompare(b.channel) || a.title.localeCompare(b.title) || a.id.localeCompare(b.id));
  });
  const [sha, setSha] = useState("");
  const [looked, setLooked] = useState<string | null>(null);
  const [thread, setThread] = useState("");
  const [threadOn, setThreadOn] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const meta = useLoad(looked, () => api.artifact(looked ?? ""));
  const linked = useLoad(threadOn, () => api.linked(threadOn ?? ""));
  const choices = tasks.value ?? [];
  const known = choices.some((t) => t.id === thread);
  const openTask = (id: string) => {
    setThread(id);
    setThreadOn(id || null);
  };
  const lookup = (e: FormEvent) => {
    e.preventDefault();
    const s = sha.trim().toLowerCase();
    if (!isSha(s)) return sayError(null, "A sha256 is 64 hex characters.");
    setLooked(s);
  };
  const upload = async (file: File | undefined) => {
    if (!file || busy) return;
    setBusy(true);
    try {
      const out = await api.upload(file);
      setSha(out.sha256);
      setLooked(out.sha256);
      say(`Uploaded ${file.name}`);
    } catch (e) {
      sayError(e, "Could not upload that file");
    } finally {
      setBusy(false);
    }
  };
  const erase = async (s: string) => {
    if (busy) return;
    setBusy(true);
    try {
      await api.eraseArtifact(s);
      say("File erased from the store");
      setLooked("");
    } catch (e) {
      sayError(e, "Could not erase that file");
    } finally {
      setBusy(false);
    }
  };
  const linkOp = async (op: "link" | "unlink", s: string) => {
    if (!threadOn || busy) return;
    setBusy(true);
    try {
      await (op === "link" ? api.link(threadOn, s) : api.unlink(threadOn, s));
      say(op === "link" ? "Linked to the task as evidence" : "Unlinked");
      await linked.reload();
    } catch (e) {
      sayError(e, op === "link" ? "Could not link that file" : "Could not unlink that file");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="tool-grid">
      <Library />
      <section className="card tool-panel" aria-labelledby="find-file-heading">
        <h2 id="find-file-heading">Find a file</h2>
        <form className="row-form" onSubmit={lookup}>
          <label className="field grow">sha256<input value={sha} onChange={(e) => setSha(e.target.value)} spellCheck={false} autoCapitalize="off" placeholder="64 hex characters" /></label>
          <button className="button steel">Look up</button>
        </form>
        <label className="button ghost file-pick">
          {busy ? "Working…" : "Upload a file"}
          <input type="file" className="visually-hidden" disabled={busy} onChange={(e) => void upload(e.target.files?.[0])} />
        </label>
        {looked ? (
          meta.status === "loading" ? <p className="muted">Looking up…</p> : meta.value ? <ArtifactCard api={api} sha={looked} meta={meta.value} onLink={threadOn ? () => void linkOp("link", looked) : null} onErase={() => void erase(looked)} busy={busy} /> : meta.error ? <p className="err" role="alert">{meta.error}</p> : <p className="muted">Not found, or not in this workspace.</p>
        ) : null}
      </section>
      <section className="card tool-panel" aria-labelledby="task-evidence-heading">
        <h2 id="task-evidence-heading">A task's evidence</h2>
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); setThreadOn(thread.trim() || null); }}>
          <label className="field grow">Choose a task
            <select aria-label="Choose a task" value={known ? thread : ""} onChange={(e) => openTask(e.target.value)}>
              <option value="">Choose a task</option>
              {choices.map((t) => <option key={t.id} value={t.id}>{taskOptionLabel(t.channel, t.title)}</option>)}
            </select>
          </label>
          <label className="field grow">Task id<input value={thread} onChange={(e) => setThread(e.target.value)} spellCheck={false} /></label>
          <button className="button steel" disabled={!thread.trim()}>Open</button>
        </form>
        {threadOn ? (
          linked.status === "loading" ? <p className="muted">Loading this task's files…</p> : linked.value === null ? <p className="err" role="alert">{linked.error}</p> : linked.value.length ? (
            <EvidenceList threadId={threadOn} shas={linked.value} api={api} busy={busy} onOpen={(s) => { setSha(s); setLooked(s); }} onUnlink={(s) => void linkOp("unlink", s)} />
          ) : (
            <p className="muted">No files linked to this task yet. Look one up, then link it.</p>
          )
        ) : <p className="muted small">Open a task to see and change the files linked to it as evidence.</p>}
      </section>
    </div>
  );
}

function EvidenceList({ threadId, shas, api, busy, onOpen, onUnlink }: { threadId: string; shas: string[]; api: ToolsApi; busy: boolean; onOpen: (sha: string) => void; onUnlink: (sha: string) => void }) {
  const key = `${threadId}:${shas.join("\n")}`;
  const metas = useLoad(key, async () => {
    const pairs = await mapLimited(shas, 6, async (s) => [s, await api.artifact(s).catch(() => null)] as const);
    return new Map(pairs);
  });
  return (
    <ul className="tool-list">
      {shas.map((s) => {
        const meta = metas.value?.get(s) ?? null;
        const named = meta?.filename?.trim() || "";
        const size = formatBytes(meta?.size_bytes);
        return (
          <li key={s} tabIndex={-1} className="tool-row">
            <button type="button" className="link-btn grow" onClick={() => onOpen(s)}>
              <span>{named || `${s.slice(0, 16)}…`}</span>
              {size ? <span className="muted small"> {size}</span> : null}
              {named ? <span className="mono-id"> {s.slice(0, 12)}…</span> : null}
            </button>
            <a href={`#/threads/t/${encodeURIComponent(threadId)}`}>Open task</a>
            <button type="button" className="chip-btn" disabled={busy} onClick={() => onUnlink(s)}>Unlink</button>
          </li>
        );
      })}
    </ul>
  );
}

function ArtifactCard({ api, sha, meta, onLink, onErase, busy }: { api: ToolsApi; sha: string; meta: ArtifactMeta; onLink: (() => void) | null; onErase: () => void; busy: boolean }) {
  const [armed, setArmed] = useState<string | null>(null);
  const [url, setUrl] = useState<{ sha: string; url: string } | null>(null);
  const show = previewable(meta.mime_type);
  useEffect(() => {
    if (!show) return;
    let live = true;
    let made: string | null = null;
    api.artifactBlob(sha).then(
      (b) => {
        if (!live) return;
        made = URL.createObjectURL(b);
        setUrl({ sha, url: made });
      },
      () => undefined,
    );
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [api, sha, show]);
  const download = async () => {
    try {
      const b = await api.artifactBlob(sha);
      const a = document.createElement("a");
      a.href = URL.createObjectURL(b);
      a.download = meta.filename || sha;
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 1000);
    } catch (e) {
      sayError(e, "Could not download that file");
    }
  };
  return (
    <div className="artifact">
      <dl className="facts">
        <div><dt>Name</dt><dd>{meta.filename || "No name"}</dd></div>
        <div><dt>Type</dt><dd>{meta.mime_type || "Unknown"}</dd></div>
        <div><dt>Size</dt><dd>{formatBytes(meta.size_bytes) || "Unknown"}</dd></div>
        <div><dt>sha256</dt><dd className="mono-id break">{sha}</dd></div>
      </dl>
      {url && url.sha === sha ? <img className="artifact-img" src={url.url} alt={meta.filename || "Image file"} /> : null}
      <div className="row-form">
        <button type="button" className="button steel" onClick={() => void download()}>Download</button>
        {onLink ? <button type="button" className="button ghost" disabled={busy} onClick={onLink}>Link to the open task</button> : null}
        {armed === sha ? (
          <>
            <button type="button" className="button danger" disabled={busy} onClick={() => { setArmed(null); onErase(); }}>Erase for good</button>
            <button type="button" className="button ghost" onClick={() => setArmed(null)}>Keep it</button>
          </>
        ) : (
          <button type="button" className="button ghost" disabled={busy} onClick={() => setArmed(sha)}>Erase…</button>
        )}
      </div>
      {armed === sha ? <p className="muted small" role="status">Erasing removes the bytes for every task that links them. It needs an admin token and cannot be undone.</p> : null}
    </div>
  );
}
