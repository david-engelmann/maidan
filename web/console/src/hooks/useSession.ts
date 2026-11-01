import { createContext, useContext, useSyncExternalStore } from "react";
import { SessionStore, type SessionState } from "../core/session.ts";

export const SessionContext = createContext<SessionStore | null>(null);

export function createSessionStore(base = window.location.origin): SessionStore {
  return new SessionStore(base, (url, init) => fetch(url, init));
}

/** The sign-in state, and the store that changes it. */
export function useSession(): { session: SessionState; store: SessionStore } {
  const store = useContext(SessionContext);
  if (!store) throw new Error("useSession needs a SessionContext provider");
  const session = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return { session, store };
}
