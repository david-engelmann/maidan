/** The play and pause control every moving picture on the page carries
 * (WCAG 2.2.2): a small glyph button over the picture's corner. Its name says
 * what a press will do, and changes with it, so it carries no aria-pressed. */
export function MediaToggle({ paused, onToggle, noun, className }: { paused: boolean; onToggle: () => void; noun: string; className?: string }) {
  const label = `${paused ? "Play" : "Pause"} the ${noun}`;
  return (
    <button type="button" className={`media-toggle${className ? ` ${className}` : ""}`} aria-label={label} title={label} onClick={onToggle}>
      <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" focusable="false">
        {paused ? <path d="M8.5 5.8v12.4L18.5 12z" /> : <path d="M9 6.5v11M15 6.5v11" />}
      </svg>
    </button>
  );
}
