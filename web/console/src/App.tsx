// The console at /ui: one shell, a screen per place.
import { Component, useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { SessionContext, createSessionStore } from "./hooks/useSession.ts";
import { Board } from "./board/Board.tsx";
import { NeedsYou } from "./needs/NeedsYou.tsx";
import { Threads } from "./threads/Threads.tsx";
import { People } from "./people/People.tsx";
import { Tools } from "./tools/Tools.tsx";
import { Catalog } from "./catalog/Catalog.tsx";
import { Artifacts } from "./artifacts/Artifacts.tsx";
import { Activity } from "./activity/Activity.tsx";
import { Shell } from "./shell/Shell.tsx";
import { LoadError } from "./shell/LoadError.tsx";
import { leaveRoom, notices } from "./core/notices.ts";
import type { SessionState, SessionStore } from "./core/session.ts";
import type { RouteId } from "./shell/router.ts";

type BoundaryProps = { resetKey: string; children: ReactNode };
type BoundaryState = { failed: boolean; resetKey: string };

/** One screen's error stays in that screen. The rail, palette, and sign-out keep working. */
export class ScreenBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { failed: false, resetKey: this.props.resetKey };

  static getDerivedStateFromProps(props: BoundaryProps, state: BoundaryState): BoundaryState | null {
    if (props.resetKey !== state.resetKey) return { failed: false, resetKey: props.resetKey };
    return null;
  }

  componentDidCatch(): void {
    this.setState({ failed: true });
  }

  render() {
    if (this.state.failed) {
      return <LoadError message="This screen hit an unexpected error." onRetry={() => this.setState({ failed: false })} />;
    }
    return this.props.children;
  }
}

/** The signed-in workspace and member, or null when there is nothing to keep apart. */
export function roomKeyOf(session: Pick<SessionState, "status" | "workspaceId" | "memberId">): string | null {
  return session.status === "signed-in" && session.workspaceId ? `${session.workspaceId}:${session.memberId ?? ""}` : null;
}

function screen(id: RouteId, roomKey: string, node: ReactNode) {
  const resetKey = `${id}:${roomKey}`;
  return (
    <ScreenBoundary key={resetKey} resetKey={resetKey}>
      {node}
    </ScreenBoundary>
  );
}

function Console({ store }: { store: SessionStore }) {
  const session = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const roomKey = roomKeyOf(session);
  const prev = useRef(roomKey);
  useEffect(() => {
    if (leaveRoom(prev.current, roomKey)) notices.clear();
    prev.current = roomKey;
  }, [roomKey]);
  const room = roomKey ?? "";
  return (
    <Shell
      screens={{
        board: screen("board", room, <Board />),
        needs: screen("needs", room, <NeedsYou />),
        threads: screen("threads", room, <Threads />),
        people: screen("people", room, <People />),
        activity: screen("activity", room, <Activity />),
        tools: screen("tools", room, <Tools />),
        catalog: screen("catalog", room, <Catalog />),
        artifacts: screen("artifacts", room, <Artifacts />),
      }}
    />
  );
}

export default function App() {
  const [store] = useState(createSessionStore);
  useEffect(() => {
    const unwatch = store.watchAuth();
    void store.start();
    return unwatch;
  }, [store]);
  return (
    <SessionContext.Provider value={store}>
      <Console store={store} />
    </SessionContext.Provider>
  );
}
