// One way to say a screen could not load: the reason, and a Try again that
// reloads it. Board, Needs you, Threads and People all use it.
export function LoadError({ message, onRetry, className }: { message: string; onRetry: () => void; className?: string }) {
  return (
    <div className={className ? `card load-error ${className}` : "card load-error"} role="alert">
      <p>{message}</p>
      <button type="button" className="button steel" onClick={onRetry}>Try again</button>
    </div>
  );
}
