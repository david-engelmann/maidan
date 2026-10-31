// Hash routes (#/board, #/needs, …). The server serves /console and pinned
// asset names only, so a hash keeps every screen a deep link without a
// server-side fallback route.
import { useSyncExternalStore } from "react";

export const ROUTES = [
  { id: "board", title: "Board", hint: "Tasks by state, in the channel you pick" },
  { id: "needs", title: "Needs you", hint: "Reviews, gates and blocks waiting on you" },
  { id: "threads", title: "Threads", hint: "Conversations and their results" },
  { id: "people", title: "People", hint: "Members and agents, and who is live" },
] as const;

export type RouteId = (typeof ROUTES)[number]["id"];

export function parseRoute(hash: string): RouteId {
  const id = hash.replace(/^#\/?/, "").split("/")[0];
  return ROUTES.find((r) => r.id === id)?.id ?? "board";
}

export function navigate(id: RouteId): void {
  if (parseRoute(window.location.hash) !== id) window.location.hash = `/${id}`;
}

function subscribe(fn: () => void): () => void {
  window.addEventListener("hashchange", fn);
  return () => window.removeEventListener("hashchange", fn);
}

export function useRoute(): RouteId {
  return useSyncExternalStore(subscribe, () => parseRoute(window.location.hash));
}
