// What a reviewer reads before deciding, inline in the review pane: the
// result itself, a look inside each file, and the last few messages.
import { useEffect, useMemo, useState } from "react";
import { apiPath, type Credentials } from "../core/api.ts";
import { BoardApi, type Member } from "../core/board.ts";
import { authorLabel, freshRoster } from "../core/people.ts";
import { ThreadsApi, type Message } from "../core/threads.ts";
import { mediaEssence, previewable, type ArtifactMeta, type ToolsApi } from "../core/tools.ts";
import { useSession } from "../hooks/useSession.ts";
import { useToolsApi } from "../hooks/useTools.ts";
import { MessageBody } from "../threads/MessageBody.tsx";

const RECENT = 3;
const LOG_LINES = 12;
const TEXT_MAX = 2 * 1024 * 1024;

/** The result as text the message renderer can show; structured results show as JSON. */
export function resultText(r: unknown): string | null {
  if (r === null || r === undefined) return null;
  if (typeof r === "string") return r;
  if (typeof r === "object") {
    const o = r as Record<string, unknown>;
    // A one-line summary can read as clean when the status says otherwise,
    // so the status leads, and a rendered review body wins over the summary.
    const status = typeof o.status === "string" && o.status ? `Status: ${o.status}\n\n` : "";
    for (const k of ["rendered", "body", "summary", "text", "output", "message"]) if (typeof o[k] === "string" && o[k]) return status + (o[k] as string);
  }
  return "```\n" + JSON.stringify(r, null, 2) + "\n```";
}

/** Text a reviewer can skim: text types, JSON, and common log and note names. */
export function textLike(meta: ArtifactMeta): boolean {
  const t = mediaEssence(meta.mime_type);
  if (t.startsWith("text/") || t === "application/json" || t === "application/x-ndjson") return true;
  return /\.(log|txt|md|json|ndjson|csv|out|err|diff|patch)$/i.test(meta.filename ?? "");
}

const IMAGE_EXT: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp" };

/** The raster type a file previews as, by its type or else its name; SVG never previews. */
export function imageType(meta: ArtifactMeta): string | null {
  if (previewable(meta.mime_type)) return mediaEssence(meta.mime_type);
  const t = mediaEssence(meta.mime_type);
  if (t && t !== "application/octet-stream") return null;
  return IMAGE_EXT[(meta.filename ?? "").split(".").pop()?.toLowerCase() ?? ""] ?? null;
}

export function ReviewContext({ tid, shas }: { tid: string; shas: string[] }) {
  const { store, session } = useSession();
  const tools = useToolsApi();
  const apis = useMemo(() => {
    const f = (url: string, init?: RequestInit) => fetch(url, init);
    const creds = () => store.getSnapshot().credentials;
    const mode = () => store.getSnapshot().mode;
    return { threads: new ThreadsApi(f, creds, mode), board: new BoardApi(f, creds, mode) };
  }, [store]);
  const [result, setResult] = useState<{ tid: string; text: string | null; error: string | null } | null>(null);
  const [recent, setRecent] = useState<{ tid: string; rows: Message[]; error: string | null } | null>(null);
  // Bumped by Try again, so a failed read is never shown as an empty one.
  const [attempt, setAttempt] = useState(0);
  // Reset when the workspace changes, during render, so an author is never
  // "Someone" for a roster that has not answered yet. With no workspace there
  // is nothing to load, so the roster starts ready instead of stuck on "…".
  const workspaceId = session.workspaceId;
  const [roster, setRoster] = useState<{ workspaceId: string | null; members: Map<string, Member>; ready: boolean }>({
    ...freshRoster(workspaceId),
    members: new Map(),
  });
  const staleRoster = roster.workspaceId !== workspaceId;
  if (staleRoster) setRoster({ ...freshRoster(workspaceId), members: new Map() });
  const members = staleRoster ? new Map<string, Member>() : roster.members;
  const membersReady = staleRoster ? !workspaceId : roster.ready;

  useEffect(() => {
    let live = true;
    void apis.threads.resultRead(tid).then((r) => live && setResult({ tid, text: resultText(r.result?.result), error: r.error }));
    void apis.threads.messages(tid).then(
      (ms) => live && setRecent({ tid, rows: ms.filter((m) => !m.tombstoned_at && m.body.trim()).slice(-RECENT), error: null }),
      (e) => live && setRecent({ tid, rows: [], error: e instanceof Error ? e.message : "Could not load the messages" }),
    );
    return () => {
      live = false;
    };
  }, [apis, tid, attempt]);

  useEffect(() => {
    if (!workspaceId) return;
    let live = true;
    void apis.board.members(workspaceId).then(
      (ms) => {
        if (!live) return;
        setRoster({ workspaceId, members: new Map(ms.map((m) => [m.id, m])), ready: true });
      },
      () => {
        if (live) setRoster((r) => (r.workspaceId === workspaceId ? { ...r, ready: true } : r));
      },
    );
    return () => {
      live = false;
    };
  }, [apis, workspaceId]);

  const res = result?.tid === tid ? result : null;
  const msgs = recent?.tid === tid ? recent.rows : null;
  return (
    <div className="review-context">
      <section className="rc-section rc-result" aria-label="Result">
        <h4 className="rc-head">Result</h4>
        {!res ? (
          <p className="muted small">Loading the result…</p>
        ) : res.error ? (
          <p className="small err" role="alert">
            {res.error.replace(/\.$/, "")}. <button type="button" className="link-btn" onClick={() => setAttempt((n) => n + 1)}>Try again</button>
          </p>
        ) : res.text ? (
          <div className="rc-body msg-body">
            <MessageBody body={res.text} />
          </div>
        ) : (
          <p className="muted small">No result recorded yet.</p>
        )}
      </section>
      {shas.length > 0 && (
        <section className="rc-section rc-files" aria-label="Files">
          <h4 className="rc-head">Files</h4>
          <ul className="rc-file-list">
            {shas.map((sha) => (
              <FilePreview key={sha} api={tools} sha={sha} creds={session.credentials} />
            ))}
          </ul>
        </section>
      )}
      <section className="rc-section rc-recent" aria-label="Recent messages">
        <h4 className="rc-head">Recent messages</h4>
        {!msgs ? (
          <p className="muted small">Loading…</p>
        ) : recent?.error ? (
          <p className="small err" role="alert">
            {recent.error.replace(/\.$/, "")}. <button type="button" className="link-btn" onClick={() => setAttempt((n) => n + 1)}>Try again</button>
          </p>
        ) : msgs.length ? (
          <ol className="rc-msgs">
            {msgs.map((m) => (
              <li key={m.id} className="rc-msg">
                <p className="rc-msg-head">
                  <span className="rc-author">{authorLabel(membersReady, members.get(m.author_id)?.name)}</span>
                  <time dateTime={m.posted_at}>{when(m.posted_at)}</time>
                </p>
                <div className="rc-msg-body msg-body">
                  <MessageBody body={m.body} />
                </div>
              </li>
            ))}
          </ol>
        ) : (
          <p className="muted small">No messages yet.</p>
        )}
      </section>
    </div>
  );
}

function when(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

function FilePreview({ api, sha, creds }: { api: ToolsApi; sha: string; creds: Credentials }) {
  const [meta, setMeta] = useState<ArtifactMeta | null | undefined>(undefined);
  const [img, setImg] = useState<string | null>(null);
  const [head, setHead] = useState<string | null>(null);
  const [missing, setMissing] = useState(false);
  useEffect(() => {
    let live = true;
    let made: string | null = null;
    void api.artifact(sha).then(
      async (m) => {
        if (!live) return;
        setMeta(m);
        if (!m) return;
        const image = imageType(m);
        const text = !image && textLike(m) && (m.size_bytes ?? 0) <= TEXT_MAX;
        if (!image && !text) return;
        try {
          const b = await api.artifactBlob(sha);
          if (!live) return;
          if (image) {
            made = URL.createObjectURL(b.type === image ? b : new Blob([b], { type: image }));
            setImg(made);
          } else {
            setHead((await b.text()).split("\n").slice(0, LOG_LINES).join("\n"));
          }
        } catch {
          if (live) setMissing(true);
        }
      },
      () => live && setMeta(null),
    );
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [api, sha]);
  const name = meta?.filename || "Untitled file";
  return (
    <li className="rc-file" data-sha={sha}>
      <p className="rc-file-name">
        {meta === undefined ? (
          "Loading…"
        ) : meta !== null ? (
          <a className="msg-link" href={apiPath(creds, `/artifacts/${encodeURIComponent(sha)}`)} target="_blank" rel="noreferrer">{name}</a>
        ) : (
          name
        )}
      </p>
      {img && <img className="rc-thumb" src={img} alt={name} />}
      {missing && <p className="muted small">The file's contents could not be loaded.</p>}
      {head !== null && <pre className="rc-log msg-pre" tabIndex={0}><code>{head}</code></pre>}
    </li>
  );
}
