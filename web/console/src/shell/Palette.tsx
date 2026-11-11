// The command palette: ⌘K / Ctrl+K anywhere, or the search button. It is a
// combobox over a listbox: arrows move the active option, Enter runs it,
// Escape closes, Tab stays inside, and focus goes back where it came from.
// Signed in, the same list also finds channels, people, task titles and messages.
import { motion, useReducedMotion } from "motion/react";
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { followChannelJump } from "../core/boardQuery.ts";
import { BoardApi } from "../core/board.ts";
import { liveBus } from "../core/bus.ts";
import { sayError } from "../core/notices.ts";
import { PeopleApi } from "../core/people.ts";
import { ThreadsApi } from "../core/threads.ts";
import { ToolsApi, searchQuery, type SearchHit } from "../core/tools.ts";
import { useSession } from "../hooks/useSession.ts";
import { navigate, useRoute } from "./router.ts";
import type { Command } from "./commands.ts";
import {
  PALETTE_SEARCH_DEBOUNCE_MS,
  PALETTE_SEARCH_MIN,
  loadPaletteIndex,
  openThreadHash,
  paletteRefreshesOn,
  paletteEmpty,
  paletteItems,
  palettePlaceholder,
  peekPaletteIndex,
  storePaletteIndex,
  type PaletteIndex,
} from "./paletteIndex.ts";

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
  const { session, store } = useSession();
  const route = useRoute();
  const signedIn = session.status === "signed-in";
  const workspaceId = signedIn ? session.workspaceId : null;
  const me = signedIn ? session.memberId : null;
  const apis = useMemo(() => {
    const creds = () => store.getSnapshot().credentials;
    const mode = () => store.getSnapshot().mode;
    const fetchFn = (url: string, init?: RequestInit) => fetch(url, init);
    return {
      tools: new ToolsApi(fetchFn, creds, mode),
      people: new PeopleApi(fetchFn, creds, mode),
      board: new BoardApi(fetchFn, creds, mode),
      threads: new ThreadsApi(fetchFn, creds, mode),
    };
  }, [store]);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const [index, setIndex] = useState<PaletteIndex | null>(() => (workspaceId ? peekPaletteIndex(workspaceId) : null));
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [searching, setSearching] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  useEffect(() => input.current?.focus(), []);

  // Channels and people once per open, then again on a resync. The last index
  // for this workspace shows immediately, so a reopen is not a blank list.
  useEffect(() => {
    if (!workspaceId) return;
    let alive = true;
    let ticket = 0;
    const refresh = () => {
      const mine = ++ticket;
      const keepTasks = () => (peekPaletteIndex(workspaceId)?.tasks ?? []);
      return loadPaletteIndex(workspaceId, {
        channels: () => apis.tools.channels(workspaceId),
        people: () => apis.people.people(workspaceId),
        tasks: (channelId) => apis.board.threads(channelId).then((rows) => rows.map((t) => ({ id: t.id, title: t.title ?? "" }))),
      }, (directory) => {
        if (!alive || mine !== ticket) return;
        const next = { ...directory, tasks: keepTasks() };
        storePaletteIndex(next);
        setIndex(next);
      })
        .then((next) => {
          if (!alive || mine !== ticket) return;
          storePaletteIndex(next);
          setIndex(next);
        })
        .catch(() => {
          /* loadPaletteIndex already swallows a refused read */
        });
    };
    setIndex(peekPaletteIndex(workspaceId));
    void refresh();
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = liveBus.listen((m) => {
      const kind = m.type === "event" ? m.frame.kind : undefined;
      if (m.type !== "resync" && !paletteRefreshesOn(kind)) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        void refresh();
      }, 250);
    });
    return () => {
      alive = false;
      off();
      if (timer) clearTimeout(timer);
    };
  }, [workspaceId, apis]);

  // Message search waits a beat, and a newer query or a close drops the answer.
  useEffect(() => {
    const q = query.trim();
    if (!workspaceId || q.length < PALETTE_SEARCH_MIN) {
      setHits([]);
      setSearching(false);
      return;
    }
    setSearching(true);
    setHits([]);
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void apis.tools.search(workspaceId, searchQuery(q, "lexical", "", "")).then(
        (rows) => {
          if (cancelled) return;
          setHits(rows);
          setSearching(false);
        },
        () => {
          if (cancelled) return;
          setHits([]);
          setSearching(false);
        },
      );
    }, PALETTE_SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query, workspaceId, apis]);

  const shown = useMemo(
    () =>
      paletteItems({
        commands,
        query,
        channels: index?.workspaceId === workspaceId ? index.channels : [],
        people: index?.workspaceId === workspaceId ? index.people : [],
        tasks: index?.workspaceId === workspaceId ? index.tasks : [],
        hits,
        me,
        jumpChannel: (id) => followChannelJump(route, id),
        openPerson: (id) => {
          if (!workspaceId || !me) return;
          void apis.threads.openDm(workspaceId, me, id).then(
            (dm) => {
              window.location.hash = `#/threads/dm/${encodeURIComponent(dm.id)}`;
            },
            (e: unknown) => sayError(e, "Could not open that conversation"),
          );
        },
        openSelf: () => navigate("people"),
        openThread: (id) => {
          window.location.hash = openThreadHash(id);
        },
      }),
    [commands, query, index, hits, me, workspaceId, route, apis],
  );
  const at = Math.min(active, Math.max(0, shown.length - 1));
  const activeId = shown[at] ? `cmd-${shown[at].id}` : undefined;
  useEffect(() => {
    if (activeId) list.current?.querySelector(`#${CSS.escape(activeId)}`)?.scrollIntoView({ block: "nearest" });
  }, [activeId]);
  const run = (c: Command | undefined) => {
    if (!c) return;
    onClose();
    c.run();
  };
  const move = (by: number) => setActive((at + by + shown.length) % Math.max(1, shown.length));
  const empty = paletteEmpty(query, shown.length, searching);
  const placeholder = palettePlaceholder(signedIn);
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
          placeholder={placeholder}
          aria-label={placeholder}
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
            {(i === 0 || shown[i - 1].group !== c.group) && (
              <li role="presentation" className="palette-group">{c.group}</li>
            )}
            {/* eslint-disable-next-line jsx-a11y/click-events-have-key-events -- the keyboard picks from the field, through aria-activedescendant and Enter. */}
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
      {empty === "searching" && <p className="palette-empty" role="status">Searching…</p>}
      {empty === "none" && <p className="palette-empty" role="status">Nothing matches “{query.trim()}”.</p>}
      <p className="palette-foot" aria-hidden="true"><kbd>↑</kbd><kbd>↓</kbd> move <kbd>↵</kbd> run <kbd>G</kbd> then a letter jumps</p>
    </>
  );
}
