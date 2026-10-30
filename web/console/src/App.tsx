// The console. Each place is a placeholder until its screen lands; the
// classic board at /ui stays served until they all have.
import { useEffect, useState } from "react";
import { SessionContext, createSessionStore } from "./hooks/useSession.ts";
import { Board } from "./board/Board.tsx";
import { NeedsYou } from "./needs/NeedsYou.tsx";
import { Threads } from "./threads/Threads.tsx";
import { People } from "./people/People.tsx";
import { Shell } from "./shell/Shell.tsx";

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
          needs: <NeedsYou />,
          threads: <Threads />,
          people: <People />,
        }}
      />
    </SessionContext.Provider>
  );
}
