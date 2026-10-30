// The console scaffold. Screens arrive one per change; the old board at /ui
// stays served until they all have.
import { useEffect, useState } from "react";
import { SessionContext, createSessionStore, useSession } from "./hooks/useSession.ts";
import { useLive } from "./hooks/useLive.ts";

function Status() {
  const { session } = useSession();
  const live = useLive(session);
  const who = session.status === "signed-in" ? (session.displayName ?? session.memberId) : null;
  return (
    <p className="mono" aria-live="polite">
      {session.status === "loading" ? "Checking sign-in" : who ? `Signed in · ${who} · live ${live.kind}` : "Not signed in"}
    </p>
  );
}

export default function App() {
  const [store] = useState(createSessionStore);
  useEffect(() => {
    void store.start();
  }, [store]);
  return (
    <SessionContext.Provider value={store}>
      <main className="splash">
        <div>
          <div className="mark steel" aria-hidden="true" />
          <h1>Maidan console</h1>
          <p>The new console is being built here. The board is still at /ui.</p>
          <Status />
        </div>
      </main>
    </SessionContext.Provider>
  );
}
