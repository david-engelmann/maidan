import type { ReactNode } from "react";
import { Icon } from "./icons.tsx";
import type { RouteId } from "./router.ts";

export type EmptyIcon = RouteId | "search" | "check";

// One way to say a screen has nothing yet: a title, one line of help, and
// an optional next step. Board lanes, Threads, People, Activity and Files use
// it. With an icon it is the centred pane form, for a detail pane waiting on
// a pick: the place's own mark, not a blank square.
export function EmptyState({ title, body, action, icon }: { title: string; body: ReactNode; action?: ReactNode; icon?: EmptyIcon }) {
  return (
    <div className={icon ? "empty-state pane-empty" : "empty-state"} role="status">
      {icon ? <span className="empty-mark"><Icon name={icon} /></span> : null}
      <strong>{title}</strong>
      <p className="muted">{body}</p>
      {action ?? null}
    </div>
  );
}
