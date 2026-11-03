// Every file linked to a task in the workspace, as a grid or a list, with
// a preview, its metadata and the tasks it came from.
import { useEffect, useMemo, useState } from "react";
import { BoardApi } from "../core/board.ts";
import { KIND_LABEL, fileKind, fileName, mediaType, filterFiles, gatherFiles, textPreviewable, type FileRow } from "../core/files.ts";
import { convHash } from "../core/threads.ts";
import { formatBytes, previewable, type ArtifactMeta, type ToolsApi } from "../core/tools.ts";
import { useLoad, useToolsApi } from "../hooks/useTools.ts";
import { useSession } from "../hooks/useSession.ts";

const VIEW_KEY = "maidan_console_files_view";

async function eachLimited<T, R>(items: T[], n: number, f: (t: T) => Promise<R>): Promise<R[]> {
  const out: R[] = new Array(items.length);
  let i = 0;
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, async () => {
    while (i < items.length) { const k = i++; out[k] = await f(items[k]); }
  }));
  return out;
}

export function Library() {
  const api = useToolsApi();
  const { session, store } = useSession();
  const board = useMemo(() => new BoardApi((u, i) => fetch(u, i), () => store.getSnapshot().credentials, () => store.getSnapshot().mode), [store]);
  const ws = session.status === "signed-in" ? session.workspaceId : null;
  const files = useLoad(ws, async () => {
    const channels = await board.channels(ws ?? "");
    const tasks = (await eachLimited(channels, 4, (c) => board.threads(c.id))).flat();
    const linked = new Map<string, string[]>();
    await eachLimited(tasks, 6, async (t) => { linked.set(t.id, await api.linked(t.id).catch(() => [])); });
    const shas = [...new Set([...linked.values()].flat())];
    const metas = new Map<string, ArtifactMeta | null>();
    await eachLimited(shas, 6, async (s) => { metas.set(s, await api.artifact(s).catch(() => null)); });
    return gatherFiles(tasks, linked, metas);
  });
  const [view, setView] = useState<"grid" | "list">(() => (localStorage.getItem(VIEW_KEY) === "list" ? "list" : "grid"));
  const [q, setQ] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const pick = (v: "grid" | "list") => { setView(v); localStorage.setItem(VIEW_KEY, v); };
  const rows = filterFiles(files.value ?? [], q);
  const chosen = rows.find((r) => r.sha === open) ?? null;
  return (
    <section className="card tool-panel files" aria-labelledby="files-h">
      <div className="files-bar">
        <h2 id="files-h">All files</h2>
        {files.value ? <span className="muted small" role="status">{files.value.length === 1 ? "1 file" : `${files.value.length} files`}</span> : null}
        <div className="files-tools">
          <label className="field files-search"><span className="visually-hidden">Search files</span><input type="search" value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search by name, task or sha" /></label>
          <div className="seg" role="group" aria-label="View">
            <button type="button" className={view === "grid" ? "on" : ""} aria-pressed={view === "grid"} onClick={() => pick("grid")}>Grid</button>
            <button type="button" className={view === "list" ? "on" : ""} aria-pressed={view === "list"} onClick={() => pick("list")}>List</button>
          </div>
        </div>
      </div>
      {files.status === "loading" ? (
        <ul className={`files-${view}`} aria-busy="true" aria-label="Loading files">
          {[0, 1, 2, 3].map((i) => <li key={i} className="file-tile file-ghost" />)}
        </ul>
      ) : files.status === "error" ? (
        <div className="files-empty" role="alert"><p>Could not load the files.</p><p className="muted small">{files.error}</p><button type="button" className="button steel" onClick={() => void files.reload()}>Try again</button></div>
      ) : !files.value?.length ? (
        <div className="files-empty"><p>No files yet.</p><p className="muted small">Files show here once an agent or a person links one to a task as evidence.</p></div>
      ) : !rows.length ? (
        <div className="files-empty"><p>No files match “{q.trim()}”.</p><button type="button" className="button ghost" onClick={() => setQ("")}>Clear search</button></div>
      ) : (
        <ul className={`files-${view}`}>
          {rows.map((f) => <FileTile key={f.sha} api={api} file={f} list={view === "list"} on={open === f.sha} onOpen={() => setOpen(open === f.sha ? null : f.sha)} />)}
        </ul>
      )}
      {chosen ? <FileDetail api={api} file={chosen} onClose={() => setOpen(null)} /> : null}
    </section>
  );
}

function useBlobUrl(api: ToolsApi, sha: string, on: boolean): string | null {
  const [url, setUrl] = useState<{ sha: string; url: string } | null>(null);
  useEffect(() => {
    if (!on) return;
    let live = true;
    let made: string | null = null;
    api.artifactBlob(sha).then((b) => { if (!live) return; made = URL.createObjectURL(b); setUrl({ sha, url: made }); }, () => undefined);
    return () => { live = false; if (made) URL.revokeObjectURL(made); };
  }, [api, sha, on]);
  return url && url.sha === sha ? url.url : null;
}

function FileTile({ api, file, list, on, onOpen }: { api: ToolsApi; file: FileRow; list: boolean; on: boolean; onOpen: () => void }) {
  const kind = fileKind(mediaType(file.meta));
  const img = useBlobUrl(api, file.sha, !list && previewable(mediaType(file.meta)));
  const [broken, setBroken] = useState(false);
  const from = file.sources.length === 1 ? file.sources[0].title : `${file.sources.length} tasks`;
  return (
    <li className={on ? "file-tile on" : "file-tile"}>
      <button type="button" className="file-open" aria-expanded={on} onClick={onOpen}>
        {list ? null : <span className="file-thumb">{img && !broken ? <img src={img} alt="" onError={() => setBroken(true)} /> : <span className="file-glyph">{KIND_LABEL[kind]}</span>}</span>}
        <span className="file-text">
          <b className="file-name">{fileName(file)}</b>
          <small className="muted">{[KIND_LABEL[kind], formatBytes(file.meta?.size_bytes)].filter(Boolean).join(" · ")}</small>
          <small className="muted file-from">From {from}</small>
        </span>
      </button>
    </li>
  );
}

function FileDetail({ api, file, onClose }: { api: ToolsApi; file: FileRow; onClose: () => void }) {
  const img = useBlobUrl(api, file.sha, previewable(mediaType(file.meta)));
  const [broken, setBroken] = useState<string | null>(null);
  const text = useLoad(textPreviewable(file.meta) ? file.sha : null, async () => (await api.artifactBlob(file.sha)).text());
  const when = file.meta?.created_at ? new Date(file.meta.created_at).toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) : null;
  return (
    <div className="file-detail" role="region" aria-label={`File ${fileName(file)}`}>
      <div className="file-detail-head">
        <h3>{fileName(file)}</h3>
        <button type="button" className="icon-btn" aria-label="Close file" onClick={onClose}>×</button>
      </div>
      {img && broken !== file.sha ? <img className="artifact-img" src={img} alt={fileName(file)} onError={() => setBroken(file.sha)} /> : null}
      {text.value ? <pre className="file-pre">{text.value}</pre> : null}
      {text.status === "error" || (broken === file.sha) ? <p className="muted small" role="status">No preview: the store does not have this file's bytes any more.</p> : null}
      <dl className="facts">
        <div><dt>Type</dt><dd>{mediaType(file.meta) || "Unknown"}</dd></div>
        <div><dt>Size</dt><dd>{formatBytes(file.meta?.size_bytes) || "Unknown"}</dd></div>
        {when ? <div><dt>Added</dt><dd>{when}</dd></div> : null}
        {file.meta?.kind ? <div><dt>Kind</dt><dd>{file.meta.kind.replace(/_/g, " ")}</dd></div> : null}
        <div><dt>sha256</dt><dd className="mono-id break">{file.sha}</dd></div>
      </dl>
      <h4 className="rc-head">From</h4>
      <ul className="file-sources">
        {file.sources.map((s) => <li key={s.id}><a href={convHash({ kind: "task", id: s.id })}>{s.title}</a></li>)}
      </ul>
    </div>
  );
}
