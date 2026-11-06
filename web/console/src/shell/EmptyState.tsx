import type { ReactNode } from "react";
import { emptyStateTitle } from "./emptyErrorChannel.ts";

// One way to say a screen has nothing yet: a title, one line of help, and
// an optional next step. Board lanes, Threads, People, Activity and Files use it.
export { emptyStateTitle } from "./emptyErrorChannel.ts";

export function EmptyState({ title, body, action }: { title: string; body: ReactNode; action?: ReactNode }) {
  const heading = emptyStateTitle(title);
  return (
    <div className="empty-state" role="status" aria-live="polite" aria-label={heading}>
      <strong>{heading}</strong>
      <p className="muted">{body}</p>
      {action ?? null}
    </div>
  );
}
