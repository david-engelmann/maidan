// The command palette: ⌘K / Ctrl+K anywhere, or the search button. Arrow
// keys move, Enter runs, Escape closes. Commands are plain data, so screens
// can add their own later.
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";

export type Command = { id: string; title: string; hint?: string; run: () => void };

export function Palette({ open, onClose, commands }: { open: boolean; onClose: () => void; commands: Command[] }) {
  const reduce = useReducedMotion();
  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="palette-scrim"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: reduce ? 0 : 0.16 }}
          onMouseDown={(e) => e.target === e.currentTarget && onClose()}
        >
          <motion.div
            className="palette"
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
            initial={reduce ? false : { y: -12, scale: 0.98, opacity: 0 }}
            animate={{ y: 0, scale: 1, opacity: 1 }}
            exit={reduce ? { opacity: 0 } : { y: -8, opacity: 0 }}
            transition={{ type: "spring", stiffness: 520, damping: 38 }}
          >
            <PaletteBody commands={commands} onClose={onClose} />
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

function PaletteBody({ commands, onClose }: { commands: Command[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? commands.filter((c) => `${c.title} ${c.hint ?? ""}`.toLowerCase().includes(q)) : commands;
  }, [commands, query]);
  const at = Math.min(active, Math.max(0, shown.length - 1));
  const run = (c: Command | undefined) => {
    if (!c) return;
    onClose();
    c.run();
  };
  return (
    <>
      <input
        ref={input}
        className="palette-input"
        placeholder="Go to, or run a command"
        aria-label="Search commands"
        aria-controls="palette-list"
        aria-activedescendant={shown[at] ? `cmd-${shown[at].id}` : undefined}
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(0);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setActive((at + 1) % Math.max(1, shown.length));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setActive((at - 1 + shown.length) % Math.max(1, shown.length));
          } else if (e.key === "Enter") {
            e.preventDefault();
            run(shown[at]);
          } else if (e.key === "Escape") {
            onClose();
          }
        }}
      />
      <ul id="palette-list" className="palette-list" role="listbox">
        {shown.map((c, i) => (
          <li
            key={c.id}
            id={`cmd-${c.id}`}
            role="option"
            aria-selected={i === at}
            className={i === at ? "active" : undefined}
            onMouseEnter={() => setActive(i)}
            onClick={() => run(c)}
          >
            <span>{c.title}</span>
            {c.hint && <span className="muted">{c.hint}</span>}
          </li>
        ))}
        {!shown.length && <li className="empty">Nothing matches</li>}
      </ul>
    </>
  );
}
