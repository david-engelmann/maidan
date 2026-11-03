// Hash routes (#/board, #/needs, …). The server serves /ui and pinned
// asset names only, so a hash keeps every screen a deep link without a
// server-side fallback route.
import { useSyncExternalStore } from "react";

export const ROUTES = [
  { id: "board", title: "Board", hint: "Tasks by state, in the channel you pick" },
  { id: "needs", title: "Needs you", hint: "Reviews, gates and blocks waiting on you" },
  { id: "threads", title: "Threads", hint: "Conversations and their results" },
  { id: "people", title: "People", hint: "Members and agents, and who is live" },
  { id: "activity", title: "Activity", hint: "Everything happening in the workspace, live" },
  { id: "tools", title: "Tools", hint: "Inbox, preferences, deliveries, commands and the operator's desk" },
  { id: "catalog", title: "Catalog", hint: "The MCP tools agents can call here, and what each one takes" },
  { id: "artifacts", title: "Files", hint: "Artifacts: look one up, upload, link to a task" },
] as const;

export type RouteId = (typeof ROUTES)[number]["id"];

/** The phone tab bar holds five places; the rest sit behind More. */
export const PHONE_TABS: readonly RouteId[] = ["board", "needs", "threads", "people", "activity"];

export function phoneTabs(): { tabs: (typeof ROUTES)[number][]; more: (typeof ROUTES)[number][] } {
  return { tabs: ROUTES.filter((r) => PHONE_TABS.includes(r.id)), more: ROUTES.filter((r) => !PHONE_TABS.includes(r.id)) };
}

/** A place, or a hash that names none. */
export type Route = RouteId | "not-found";

/** The first segment of a hash route: the place it asks for. */
export function placeOf(hash: string): string {
  return hash.replace(/^#\/?/, "").split(/[/?]/)[0];
}

// An empty hash is the board. A name that matches no place is not-found,
// not the board: a mistyped or stale link (#/files) that quietly opened the
// board looked like the link had worked.
export function parseRoute(hash: string): Route {
  if (/^#\/?confirm-approval=/.test(hash)) return "needs";
  const id = placeOf(hash);
  if (!id) return "board";
  return ROUTES.find((r) => r.id === id)?.id ?? "not-found";
}

export function navigate(id: RouteId): void {
  if (parseRoute(window.location.hash) !== id) window.location.hash = `/${id}`;
}

function subscribe(fn: () => void): () => void {
  window.addEventListener("hashchange", fn);
  return () => window.removeEventListener("hashchange", fn);
}

/** The place the hash asks for, current as the hash changes. */
export function usePlace(): string {
  return useSyncExternalStore(subscribe, () => placeOf(window.location.hash));
}

export function useRoute(): Route {
  return useSyncExternalStore(subscribe, () => parseRoute(window.location.hash));
}
