// The console. Each place is a placeholder until its screen lands; the
// classic board at /ui stays served until they all have.
import { useEffect, useState } from "react";
import { SessionContext, createSessionStore, useSession } from "./hooks/useSession.ts";
import { Board } from "./board/Board.tsx";
import { Shell } from "./shell/Shell.tsx";
import { ROUTES, type RouteId } from "./shell/router.ts";

function Placeholder({ id }: { id: RouteId }) {
  const { session } = useSession();
  const route = ROUTES.find((r) => r.id === id) ?? ROUTES[0];
  return (
    <div className="card">
      <p className="mono">Coming to the console</p>
      <h2>{route.title}</h2>
      <p className="muted">{route.hint}. Until it lands here, it is on the classic board.</p>
      <a className="button steel" href="/ui/">
        Open the classic board
      </a>
      {session.status === "signed-out" && <p className="muted small">You are not signed in. Sign in on the classic board first.</p>}
    </div>
  );
}

export default function App() {
  const [store] = useState(createSessionStore);
  useEffect(() => {
    void store.start();
  }, [store]);
  return (
    <SessionContext.Provider value={store}>
      <Shell
        screens={{
          board: <Board />,
          needs: <Placeholder id="needs" />,
          threads: <Placeholder id="threads" />,
          people: <Placeholder id="people" />,
        }}
      />
    </SessionContext.Provider>
  );
}
