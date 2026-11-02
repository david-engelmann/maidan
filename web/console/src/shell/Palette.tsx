// The command palette: ⌘K / Ctrl+K anywhere, or the search button. It is a
// combobox over a listbox: arrows move the active option, Enter runs it,
// Escape closes, Tab stays inside, and focus goes back where it came from.
import { motion, useReducedMotion } from "motion/react";
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { rank, type Command } from "./commands.ts";

export type { Command };

export function Palette({ open, onClose, commands }: { open: boolean; onClose: () => void; commands: Command[] }) {
  const reduce = useReducedMotion();
  // Remember what had focus when the palette opened and hand it back on close.
  const opener = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    if (open) {
      opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      return;
    }
    const el = opener.current;
    opener.current = null;
    if (el && el.isConnected) el.focus();
  }, [open]);
  // No exit animation: closing is instant, so a quick reopen always starts
  // fresh with the field focused.
  if (!open) return null;
  return (
        <motion.div
          className="palette-scrim"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: reduce ? 0 : 0.14 }}
          onMouseDown={(e) => e.target === e.currentTarget && onClose()}
        >
          <motion.div
            className="palette"
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
            initial={reduce ? false : { y: -10, scale: 0.98, opacity: 0 }}
            animate={{ y: 0, scale: 1, opacity: 1 }}
            transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 560, damping: 40 }}
          >
            <PaletteBody commands={commands} onClose={onClose} />
          </motion.div>
        </motion.div>
  );
}

function PaletteBody({ commands, onClose }: { commands: Command[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  useEffect(() => input.current?.focus(), []);
  const shown = useMemo(() => rank(commands, query), [commands, query]);
  const at = Math.min(active, Math.max(0, shown.length - 1));
  const activeId = shown[at] ? `cmd-${shown[at].id}` : undefined;
  useEffect(() => {
    if (activeId) list.current?.querySelector(`#${activeId}`)?.scrollIntoView({ block: "nearest" });
  }, [activeId]);
  const run = (c: Command | undefined) => {
    if (!c) return;
    onClose();
    c.run();
  };
  const move = (by: number) => setActive((at + by + shown.length) % Math.max(1, shown.length));
  return (
    <>
      <div className="palette-field">
        <input
          ref={input}
          className="palette-input"
          type="text"
          role="combobox"
          aria-expanded="true"
          aria-autocomplete="list"
          aria-haspopup="listbox"
          placeholder="Go to a place or run a command…"
          aria-label="Search commands"
          aria-controls="palette-list"
          aria-activedescendant={activeId}
          autoComplete="off"
          spellCheck={false}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) {
              e.preventDefault();
              move(1);
            } else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) {
              e.preventDefault();
              move(-1);
            } else if (e.key === "Home" && shown.length) {
              e.preventDefault();
              setActive(0);
            } else if (e.key === "End" && shown.length) {
              e.preventDefault();
              setActive(shown.length - 1);
            } else if (e.key === "Enter") {
              e.preventDefault();
              run(shown[at]);
            } else if (e.key === "Escape") {
              e.preventDefault();
              onClose();
            } else if (e.key === "Tab") {
              e.preventDefault();
            }
          }}
        />
        <kbd className="palette-esc" aria-hidden="true">esc</kbd>
      </div>
      <ul id="palette-list" ref={list} className="palette-list" role="listbox" aria-label="Commands">
        {shown.map((c, i) => (
          <Fragment key={c.id}>
            {!query.trim() && (i === 0 || shown[i - 1].group !== c.group) && (
              <li role="presentation" className="palette-group">{c.group}</li>
            )}
            <li
              id={`cmd-${c.id}`}
              role="option"
              aria-selected={i === at}
              className={i === at ? "active" : undefined}
              onMouseMove={() => i !== at && setActive(i)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => run(c)}
            >
              <span className="palette-text">
                <span className="palette-title">{c.title}</span>
                {c.hint && <span className="palette-hint">{c.hint}</span>}
              </span>
              {c.keys && (
                <span className="palette-keys" aria-hidden="true">
                  {c.keys.map((k) => <kbd key={k}>{k}</kbd>)}
                </span>
              )}
            </li>
          </Fragment>
        ))}
      </ul>
      {!shown.length && <p className="palette-empty" role="status">Nothing matches “{query.trim()}”.</p>}
      <p className="palette-foot" aria-hidden="true"><kbd>↑</kbd><kbd>↓</kbd> move <kbd>↵</kbd> run <kbd>G</kbd> then a letter jumps</p>
    </>
  );
}
