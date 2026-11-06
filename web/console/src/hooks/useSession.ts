import { createContext, useContext, useSyncExternalStore } from "react";
import { SessionStore, type SessionState } from "../core/session.ts";

export const SessionContext = createContext<SessionStore | null>(null);

export function createSessionStore(base = typeof window !== "undefined" ? window.location.origin : ""): SessionStore {
  return new SessionStore(base, (url, init) => fetch(url, init));
}

/** The sign-in state, and the store that changes it. */
export function useSession(): { session: SessionState; store: SessionStore } {
  const store = useContext(SessionContext);
  if (!store) throw new Error("useSession needs a SessionContext provider");
  // Same snapshot on the server and the client avoids a missing getServerSnapshot warning
  // and keeps the first paint aligned with the store's current value.
  const session = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return { session, store };
}
