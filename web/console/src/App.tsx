// The console at /ui: one shell, a screen per place.
import { useEffect, useState } from "react";
import { SessionContext, createSessionStore } from "./hooks/useSession.ts";
import { Board } from "./board/Board.tsx";
import { NeedsYou } from "./needs/NeedsYou.tsx";
import { Threads } from "./threads/Threads.tsx";
import { People } from "./people/People.tsx";
import { Tools } from "./tools/Tools.tsx";
import { Artifacts } from "./artifacts/Artifacts.tsx";
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
          tools: <Tools />,
          artifacts: <Artifacts />,
        }}
      />
    </SessionContext.Provider>
  );
}
