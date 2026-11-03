import type { ReactNode } from "react";

// One way to say a screen has nothing yet: a title, one line of help, and
// an optional next step. Board lanes, Threads, People, Activity and Files use it.
export function EmptyState({ title, body, action }: { title: string; body: ReactNode; action?: ReactNode }) {
  return (
    <div className="empty-state" role="status">
      <strong>{title}</strong>
      <p className="muted">{body}</p>
      {action ?? null}
    </div>
  );
}
