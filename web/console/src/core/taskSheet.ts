// The task sheet's reads that the board client does not own: a task's
// transition history and the tasks it waits on. Fetch is passed in, and the
// paths go through apiPath, so a session and a token both hit the route
// they are allowed to call. Kept out of the board module so a sheet change
// does not collide with board work.
import { apiPath, headers, type Credentials } from "./api.ts";
import { TRANSITION_WORDS, type Fetch, type Transition } from "./board.ts";
import { convHash } from "./threads.ts";

export type HistoryTransition = Transition & { occurred_at?: string | null };

/** When a transition happened. The route sends `occurred_at`; older rows used `at`. */
export function transitionAt(row: HistoryTransition): string | null {
  const at = row.occurred_at || row.at || row.created_at || "";
  return at || null;
}

/** Newest first. Rows with no time stay at the end, in their original order. */
export function historyNewestFirst<T extends HistoryTransition>(rows: readonly T[]): T[] {
  return rows.map((row, index) => ({ row, index })).sort((a, b) => {
    const aAt = transitionAt(a.row) ?? "";
    const bAt = transitionAt(b.row) ?? "";
    if (!aAt || !bAt) {
      if (aAt === bAt) return a.index - b.index;
      return aAt ? -1 : 1;
    }
    if (aAt !== bAt) return bAt.localeCompare(aAt);
    return a.index - b.index;
  }).map((x) => x.row);
}

/**
 * What a transition did, in the words the sheet's own actions use.
 * A state with no action word is shown as the state the task landed in.
 */
export function transitionPhrase(row: { to_state?: string | null }): string {
  switch (row.to_state) {
    case "in_review":
      return TRANSITION_WORDS.start_review;
    case "closed":
      return TRANSITION_WORDS.close;
    case "archived":
      return TRANSITION_WORDS.archive;
    case "open":
      return "Open";
    default: {
      const word = (row.to_state ?? "").replace(/_/g, " ").trim();
      return word || "Updated";
    }
  }
}

/** How many dependency reads run at once. */
export const DEP_FETCH_LIMIT = 4;

/** Run `fn` over `items` with at most `limit` calls in flight, results in order. */
export async function mapBounded<T, R>(items: readonly T[], limit: number, fn: (item: T) => Promise<R>): Promise<R[]> {
  if (!items.length) return [];
  const out: R[] = new Array(items.length);
  let cursor = 0;
  const workers = Math.max(1, Math.min(Math.floor(limit) || 1, items.length));
  await Promise.all(
    Array.from({ length: workers }, async () => {
      for (;;) {
        const i = cursor++;
        if (i >= items.length) return;
        out[i] = await fn(items[i] as T);
      }
    }),
  );
  return out;
}

/** Dependency ids from `GET /threads/{id}/dependencies`, in wire order, duplicates dropped. */
export function dependencyIds(body: unknown): string[] {
  const rows = body && typeof body === "object" && Array.isArray((body as { dependencies?: unknown }).dependencies)
    ? (body as { dependencies: unknown[] }).dependencies
    : [];
  const out: string[] = [];
  const seen = new Set<string>();
  for (const row of rows) {
    const id = row && typeof row === "object" ? (row as { depends_on_thread_id?: unknown }).depends_on_thread_id : undefined;
    if (typeof id !== "string" || !id || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}

export type DepThread = { title?: string | null; state?: string };

/** The title a dependency link shows. A missing task never falls back to its id. */
export function dependencyLabel(th: DepThread | null): string {
  const title = th?.title?.trim();
  if (title) return title;
  return th ? "Untitled task" : "Unavailable task";
}

/** A short state cue beside a dependency title. */
export function dependencyCue(state: string | null | undefined): string {
  if (state === "closed" || state === "archived") return "(done)";
  if (state === "in_review") return "(in review)";
  if (state === "open") return "(open)";
  if (!state) return "";
  return `(${state.replace(/_/g, " ")})`;
}

export type DepLink = { id: string; title: string; cue: string; href: string };

function asDepThread(body: unknown): DepThread | null {
  if (!body || typeof body !== "object") return null;
  const o = body as { title?: unknown; state?: unknown };
  return {
    title: typeof o.title === "string" ? o.title : null,
    state: typeof o.state === "string" ? o.state : undefined,
  };
}

async function readJson(fetchFn: Fetch, creds: Credentials, path: string): Promise<unknown> {
  const res = await fetchFn(apiPath(creds, path), { headers: headers(creds), credentials: "include" });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json();
}

/**
 * The tasks this one waits on. An unreadable dependency stays in the list
 * with a plain label, so a deleted task does not hide the ones beside it.
 * A failed list read is no dependencies.
 */
export async function loadTaskDependencies(fetchFn: Fetch, creds: Credentials, tid: string): Promise<DepLink[]> {
  let ids: string[];
  try {
    ids = dependencyIds(await readJson(fetchFn, creds, `/threads/${tid}/dependencies`));
  } catch {
    return [];
  }
  if (!ids.length) return [];
  const threads = await mapBounded(ids, DEP_FETCH_LIMIT, async (id) => {
    try {
      return asDepThread(await readJson(fetchFn, creds, `/threads/${id}`));
    } catch {
      return null;
    }
  });
  return ids.map((id, i) => {
    const th = threads[i] ?? null;
    return { id, title: dependencyLabel(th), cue: dependencyCue(th?.state), href: convHash({ kind: "task", id }) };
  });
}
