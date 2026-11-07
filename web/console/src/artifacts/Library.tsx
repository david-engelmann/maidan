// Every file linked to a task in the workspace, as a grid or a list, with
// a preview, its metadata and the tasks it came from.
import { useEffect, useMemo, useState } from "react";
import { BoardApi } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";
import { FILE_REFRESH_KINDS, KIND_LABEL, fileKind, fileName, mediaType, filterFiles, gatherFiles, textPreviewable, type FileRow } from "../core/files.ts";
import { convHash } from "../core/threads.ts";
import { formatBytes, previewable, type ArtifactMeta, type ToolsApi } from "../core/tools.ts";
import { useLoad, useToolsApi } from "../hooks/useTools.ts";
import { useSession } from "../hooks/useSession.ts";
import { EmptyState } from "../shell/EmptyState";

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
    await eachLimited(tasks, 6, async (t) => { linked.set(t.id, await api.linked(t.id)); });
    const shas = [...new Set([...linked.values()].flat())];
    const metas = new Map<string, ArtifactMeta | null>();
    const failed = new Set<string>();
    await eachLimited(shas, 6, async (s) => {
      try {
        metas.set(s, await api.artifact(s));
      } catch {
        failed.add(s);
        metas.set(s, null);
      }
    });
    return gatherFiles(tasks, linked, metas, failed);
  });
  const reloadFiles = files.reload;
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = liveBus.listen((m) => {
      if (m.type !== "resync" && !FILE_REFRESH_KINDS.has(m.frame.kind ?? "")) return;
      if (timer) clearTimeout(timer);
      // A store event for the bytes can land a moment before the thread link.
      timer = setTimeout(() => void reloadFiles(), 300);
    });
    return () => {
      off();
      if (timer) clearTimeout(timer);
    };
  }, [reloadFiles]);
  const [view, setView] = useState<"grid" | "list">(() => (localStorage.getItem(VIEW_KEY) === "list" ? "list" : "grid"));
  const [q, setQ] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const pick = (v: "grid" | "list") => { setView(v); localStorage.setItem(VIEW_KEY, v); };
  const rows = filterFiles(files.value ?? [], q);
  const chosen = rows.find((r) => r.sha === open) ?? null;
  const metaFailed = (files.value ?? []).some((f) => f.metaFailed);
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
      {metaFailed ? (
        <p className="muted small" role="status">
          Some file details could not be loaded.{" "}
          <button type="button" className="chip-btn" onClick={() => void files.reload()}>Retry metadata</button>
        </p>
      ) : null}
      {files.status === "loading" ? (
        <ul className={`files-${view}`} aria-busy="true" aria-label="Loading files">
          {[0, 1, 2, 3].map((i) => <li key={i} className="file-tile file-ghost" />)}
        </ul>
      ) : files.status === "error" ? (
        <div className="files-empty" role="alert"><p>Could not load the files.</p><p className="muted small">{files.error}</p><button type="button" className="button steel" onClick={() => void files.reload()}>Try again</button></div>
      ) : !files.value?.length ? (
        <EmptyState title="No files yet" body="Files show here once an agent or a person links one to a task as evidence." />
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

// The URL is dropped with its revoke, so a tile back in Grid reads the bytes
// again instead of loading a dead URL; a failed read says so.
function useBlobUrl(api: ToolsApi, sha: string, on: boolean): { url: string | null; failed: boolean } {
  const [got, setGot] = useState<{ sha: string; url: string | null } | null>(null);
  useEffect(() => {
    if (!on) return;
    let live = true;
    let made: string | null = null;
    api.artifactBlob(sha).then(
      (b) => { if (!live) return; made = URL.createObjectURL(b); setGot({ sha, url: made }); },
      () => { if (live) setGot({ sha, url: null }); },
    );
    return () => { live = false; if (made) URL.revokeObjectURL(made); setGot(null); };
  }, [api, sha, on]);
  const mine = on && got?.sha === sha ? got : null;
  return { url: mine?.url ?? null, failed: mine !== null && mine.url === null };
}

function FileTile({ api, file, list, on, onOpen }: { api: ToolsApi; file: FileRow; list: boolean; on: boolean; onOpen: () => void }) {
  const kind = fileKind(mediaType(file.meta));
  const { url: img } = useBlobUrl(api, file.sha, !list && previewable(mediaType(file.meta)));
  const [broken, setBroken] = useState<string | null>(null);
  const from = file.sources.length === 1 ? file.sources[0].title : `${file.sources.length} tasks`;
  return (
    <li className={on ? "file-tile on" : "file-tile"}>
      <button type="button" className="file-open" aria-expanded={on} onClick={onOpen}>
        {list ? null : <span className="file-thumb">{img && broken !== img ? <img src={img} alt="" onError={() => setBroken(img)} /> : <span className="file-glyph">{KIND_LABEL[kind]}</span>}</span>}
        <span className="file-text">
          <b className="file-name">{fileName(file)}</b>
          <small className="muted">{file.metaFailed ? "Metadata unavailable" : [KIND_LABEL[kind], formatBytes(file.meta?.size_bytes)].filter(Boolean).join(" · ")}</small>
          <small className="muted file-from">From {from}</small>
        </span>
      </button>
    </li>
  );
}

function FileDetail({ api, file, onClose }: { api: ToolsApi; file: FileRow; onClose: () => void }) {
  const { url: img, failed } = useBlobUrl(api, file.sha, previewable(mediaType(file.meta)));
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
      {text.status === "error" || failed || broken === file.sha ? <p className="muted small" role="status">No preview: the store does not have this file's bytes any more.</p> : null}
      <dl className="facts">
        <div><dt>Type</dt><dd>{file.metaFailed ? "Metadata unavailable" : mediaType(file.meta) || "Unknown"}</dd></div>
        <div><dt>Size</dt><dd>{file.metaFailed ? "Metadata unavailable" : formatBytes(file.meta?.size_bytes) || "Unknown"}</dd></div>
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
