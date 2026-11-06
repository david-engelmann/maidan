import { loadErrorMessage } from "./emptyErrorChannel.ts";

// One way to say a screen could not load: the reason, and a Try again that
// reloads it. Board, Needs you, Threads and People all use it.
export { loadErrorMessage } from "./emptyErrorChannel.ts";

export function LoadError({ message, onRetry, className }: { message: string; onRetry: () => void; className?: string }) {
  const text = loadErrorMessage(message);
  return (
    <div className={className ? `card load-error ${className}` : "card load-error"} role="alert" aria-live="assertive">
      <p>{text}</p>
      <button type="button" className="button steel" onClick={onRetry} aria-label={`Try again: ${text}`}>
        Try again
      </button>
    </div>
  );
}
