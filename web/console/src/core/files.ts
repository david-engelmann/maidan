// Files across the workspace. There is no workspace-wide file list in the
// API, so it is gathered: every task's linked files, then each file's meta.
// A file linked to several tasks shows once, with every task it came from.
import type { ArtifactMeta } from "./tools.ts";

export type FileSource = { id: string; title: string };
export type FileRow = { sha: string; meta: ArtifactMeta | null; sources: FileSource[]; metaFailed?: boolean };
export type FileKind = "image" | "text" | "data" | "archive" | "other";

/**
 * Live frames that can change which files are linked. Storing bytes emits
 * `artifact_upserted`. Linking and unlinking a thread artifact write no event
 * of their own; the link kinds are still honored when a server sends one.
 */
export const FILE_REFRESH_KINDS = new Set(["artifact_upserted", "artifact_linked", "artifact_unlinked", "thread_artifact_linked", "thread_artifact_unlinked"]);

export function gatherFiles(
  tasks: Array<{ id: string; title?: string | null }>,
  linked: Map<string, string[]>,
  metas: Map<string, ArtifactMeta | null>,
  failedMeta?: ReadonlySet<string>,
): FileRow[] {
  const by = new Map<string, FileRow>();
  for (const t of tasks) {
    for (const sha of linked.get(t.id) ?? []) {
      const row = by.get(sha) ?? { sha, meta: metas.get(sha) ?? null, sources: [], metaFailed: failedMeta?.has(sha) ?? false };
      if (failedMeta?.has(sha)) row.metaFailed = true;
      if (!row.sources.some((s) => s.id === t.id)) row.sources.push({ id: t.id, title: t.title?.trim() || "Untitled task" });
      by.set(sha, row);
    }
  }
  return [...by.values()].sort((a, b) => String(b.meta?.created_at ?? "").localeCompare(String(a.meta?.created_at ?? "")) || fileName(a).localeCompare(fileName(b)));
}

export function fileName(f: Pick<FileRow, "sha" | "meta">): string {
  return f.meta?.filename?.trim() || `${f.sha.slice(0, 12)}…`;
}

const BY_EXT: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", txt: "text/plain", md: "text/markdown", log: "text/plain", json: "application/json", csv: "text/csv", zip: "application/zip" };

/** The media type, or one read from the name's extension when the store has none. */
export function mediaType(meta: { mime_type?: string | null; filename?: string | null } | null): string {
  const t = String(meta?.mime_type ?? "").split(";")[0].trim().toLowerCase();
  if (t && t !== "application/octet-stream") return t;
  const ext = String(meta?.filename ?? "").split(".").pop()?.toLowerCase() ?? "";
  return BY_EXT[ext] ?? t;
}

export function fileKind(type: string | null | undefined): FileKind {
  const t = String(type ?? "").split(";")[0].trim().toLowerCase();
  if (t.startsWith("image/")) return "image";
  if (t === "application/json" || t.endsWith("+json") || t === "application/xml" || t === "text/csv") return "data";
  if (t.startsWith("text/") || t === "application/markdown") return "text";
  if (/zip|tar|gzip|x-7z|compressed/.test(t)) return "archive";
  return "other";
}

export const KIND_LABEL: Record<FileKind, string> = { image: "Image", text: "Text", data: "Data", archive: "Archive", other: "File" };

export function filterFiles(rows: FileRow[], q: string): FileRow[] {
  const s = q.trim().toLowerCase();
  if (!s) return rows;
  return rows.filter((f) => fileName(f).toLowerCase().includes(s) || f.sha.startsWith(s) || f.sources.some((t) => t.title.toLowerCase().includes(s)));
}

/** Text that previews inline: small and textual. */
export function textPreviewable(meta: ArtifactMeta | null): boolean {
  const k = fileKind(mediaType(meta));
  return (k === "text" || k === "data") && (meta?.size_bytes ?? Infinity) <= 64 * 1024;
}
