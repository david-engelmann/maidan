// Files: the content-addressed artifact store. Look one up by its sha256,
// upload a file to get one, and link files to a task as its evidence.
import { type FormEvent, useEffect, useState } from "react";
import { say, sayError } from "../core/notices.ts";
import { formatBytes, isSha, previewable, type ArtifactMeta, type ToolsApi } from "../core/tools.ts";
import { useSession } from "../hooks/useSession.ts";
import { useLoad, useToolsApi } from "../hooks/useTools.ts";

export function Artifacts() {
  const { session } = useSession();
  const api = useToolsApi();
  const [sha, setSha] = useState("");
  const [looked, setLooked] = useState<string | null>(null);
  const [thread, setThread] = useState("");
  const [threadOn, setThreadOn] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const meta = useLoad(looked, () => api.artifact(looked ?? ""));
  const linked = useLoad(threadOn, () => api.linked(threadOn ?? ""));
  if (session.status === "signed-out") {
    return (
      <div className="card">
        <h2>Files</h2>
        <p className="muted">Sign in on the classic board first, then come back.</p>
        <a className="button steel" href="/ui/">Open the classic board</a>
      </div>
    );
  }
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
      <section className="card tool-panel">
        <h2>Find a file</h2>
        <form className="row-form" onSubmit={lookup}>
          <label className="field grow">sha256<input value={sha} onChange={(e) => setSha(e.target.value)} spellCheck={false} autoCapitalize="off" placeholder="64 hex characters" /></label>
          <button className="button steel">Look up</button>
        </form>
        <label className="button ghost file-pick">
          {busy ? "Working…" : "Upload a file"}
          <input type="file" className="visually-hidden" disabled={busy} onChange={(e) => void upload(e.target.files?.[0])} />
        </label>
        {looked ? (
          meta.status === "loading" ? <p className="muted">Looking up…</p> : meta.value ? <ArtifactCard api={api} sha={looked} meta={meta.value} onLink={threadOn ? () => void linkOp("link", looked) : null} /> : meta.error ? <p className="err" role="alert">{meta.error}</p> : <p className="muted">Not found, or not in this workspace.</p>
        ) : null}
      </section>
      <section className="card tool-panel">
        <h2>A task's evidence</h2>
        <form className="row-form" onSubmit={(e: FormEvent) => { e.preventDefault(); setThreadOn(thread.trim() || null); }}>
          <label className="field grow">Task id<input value={thread} onChange={(e) => setThread(e.target.value)} spellCheck={false} /></label>
          <button className="button steel" disabled={!thread.trim()}>Open</button>
        </form>
        {threadOn ? (
          linked.status === "loading" ? <p className="muted">Loading this task's files…</p> : linked.value === null ? <p className="err" role="alert">{linked.error}</p> : linked.value.length ? (
            <ul className="tool-list">
              {linked.value.map((s) => (
                <li key={s} className="tool-row">
                  <button type="button" className="link-btn mono-id grow" onClick={() => { setSha(s); setLooked(s); }}>{s.slice(0, 16)}…</button>
                  <button type="button" className="chip-btn" disabled={busy} onClick={() => void linkOp("unlink", s)}>Unlink</button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted">No files linked to this task yet. Look one up, then link it.</p>
          )
        ) : <p className="muted small">Open a task to see and change the files linked to it as evidence.</p>}
      </section>
    </div>
  );
}

function ArtifactCard({ api, sha, meta, onLink }: { api: ToolsApi; sha: string; meta: ArtifactMeta; onLink: (() => void) | null }) {
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
        {onLink ? <button type="button" className="button ghost" onClick={onLink}>Link to the open task</button> : null}
      </div>
    </div>
  );
}
