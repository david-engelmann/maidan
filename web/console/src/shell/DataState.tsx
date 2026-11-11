import type { ReactNode } from "react";
import { EmptyState, type EmptyIcon } from "./EmptyState.tsx";
import { LoadError } from "./LoadError.tsx";

/**
 * The four states every read is in. A screen asks `phaseOf` which one, then
 * renders through `DataState`, so loading, empty and failed read the same in
 * every place and their words live here, not in each screen.
 */
export type Phase = "loading" | "error" | "empty" | "ready";

/**
 * Rows already on screen outrank a later failure: they stay, and the failure
 * becomes a stale line above them. Without rows, a failure outranks loading,
 * and loading outranks empty, so a first fetch still in flight never says
 * "nothing here".
 */
export function phaseOf(read: { settled: boolean; error: string | null; count: number }): Phase {
  if (read.count > 0) return "ready";
  if (read.error) return "error";
  if (!read.settled) return "loading";
  return "empty";
}

/** The one sentence for a read that failed with no better reason. */
export function couldNotLoad(what: string): string {
  return `Could not load ${what}.`;
}

/** Skeleton rows, visible from the first frame, named for a screen reader. */
export function LoadingRows({ label, rows = 3 }: { label: string; rows?: number }) {
  return (
    <div className="state-loading" role="status" aria-label={`Loading ${label}`} aria-busy="true">
      {Array.from({ length: rows }, (_, i) => (
        <span key={i} className="skeleton row-sk" />
      ))}
    </div>
  );
}

export type EmptyCopy = { title: string; body: ReactNode; action?: ReactNode; icon?: EmptyIcon };

export function DataState({
  phase,
  label,
  error,
  onRetry,
  empty,
  loading,
  stale,
  children,
}: {
  phase: Phase;
  /** What is loading, in the words of "Loading …" and "Could not load …". */
  label: string;
  error: string | null;
  onRetry: () => void;
  empty: EmptyCopy;
  /** A screen-shaped skeleton; rows of the shared one when left out. */
  loading?: ReactNode;
  /** A failure behind rows already shown: said once, above them. */
  stale?: string | null;
  children: ReactNode;
}) {
  if (phase === "loading") return <>{loading ?? <LoadingRows label={label} />}</>;
  if (phase === "error") return <LoadError message={error ?? couldNotLoad(label)} onRetry={onRetry} />;
  if (phase === "empty") return <EmptyState title={empty.title} body={empty.body} action={empty.action} icon={empty.icon} />;
  return (
    <>
      {stale ? <p className="state-stale" role="status">{stale}</p> : null}
      {children}
    </>
  );
}
