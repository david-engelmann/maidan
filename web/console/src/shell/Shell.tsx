// The app shell, iPhone first: a glass top bar under the status bar, a tab
// bar over the home indicator, and the same four places as a rail on a wide
// screen. Safe-area insets come from viewport-fit=cover in index.html.
import { motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { controlBus, liveBus } from "../core/bus.ts";
import type { ControlFrame, EventFrame } from "../core/live.ts";
import { useLive } from "../hooks/useLive.ts";
import { useSession } from "../hooks/useSession.ts";
import { Icon } from "./icons.tsx";
import { Toasts } from "./Toasts.tsx";
import { Palette, type Command } from "./Palette.tsx";
import { ROUTES, navigate, useRoute, type RouteId } from "./router.ts";

// The shell owns the one live connection; screens hear it on the bus.
const NO_FILTER = {};
const TO_BUS = {
  onEvent: (frame: EventFrame) => liveBus.emit({ type: "event", frame }),
  onControl: (frame: ControlFrame) => controlBus.emit(frame),
  onPoll: () => liveBus.emit({ type: "resync" }),
  onRecovered: () => liveBus.emit({ type: "resync" }),
};

export function Shell({ screens }: { screens: Record<RouteId, ReactNode> }) {
  const route = useRoute();
  const { session } = useSession();
  const live = useLive(session, NO_FILTER, TO_BUS);
  const reduce = useReducedMotion();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [scrolled, setScrolled] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    };
    const onScroll = () => setScrolled(window.scrollY > 4);
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll);
    };
  }, []);

  const commands = useMemo<Command[]>(
    () => [
      ...ROUTES.map((r) => ({ id: `go-${r.id}`, title: `Go to ${r.title}`, hint: r.hint, run: () => navigate(r.id) })),
      { id: "old-board", title: "Open the classic board", hint: "/ui", run: () => window.location.assign("/ui/") },
    ],
    [],
  );

  const current = ROUTES.find((r) => r.id === route) ?? ROUTES[0];
  const who = session.status === "signed-in" ? (session.displayName ?? session.memberId) : null;
  const liveWord =
    live.kind === "connected" ? "Live" : live.kind === "reconnecting" ? "Reconnecting" : live.kind === "refused" ? "Refused" : "Offline";

  return (
    <div className="shell">
      <nav className="rail" aria-label="Places">
        <a className="brand" href="#/board" aria-label="Maidan console">
          <span className="mark steel" aria-hidden="true" />
          <span>Maidan</span>
        </a>
        <Tabs route={route} reduce={Boolean(reduce)} layoutId="rail-pill" />
      </nav>

      <header className={scrolled ? "top scrolled" : "top"}>
        <h1>{current.title}</h1>
        <div className="top-actions">
          <span className={`live live-${live.kind}`} title={who ? `Signed in as ${who}` : "Not signed in"}>
            <span className="dot" aria-hidden="true" />
            {liveWord}
          </span>
          <button className="icon-btn" aria-label="Search and commands (⌘K)" onClick={() => setPaletteOpen(true)}>
            <Icon name="search" />
          </button>
        </div>
      </header>

      <main className="content" id="main">
        <motion.section
          key={route}
          initial={reduce ? false : { opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.2, ease: [0.2, 0.7, 0.2, 1] }}
          aria-label={current.title}
        >
          {screens[route]}
        </motion.section>
      </main>

      <nav className="tabbar" aria-label="Places">
        <Tabs route={route} reduce={Boolean(reduce)} layoutId="tab-pill" />
      </nav>

      <Toasts />
      <Palette open={paletteOpen} onClose={() => setPaletteOpen(false)} commands={commands} />
    </div>
  );
}

function Tabs({ route, reduce, layoutId }: { route: RouteId; reduce: boolean; layoutId: string }) {
  return (
    <ul className="tabs">
      {ROUTES.map((r) => (
        <li key={r.id}>
          <a href={`#/${r.id}`} aria-current={r.id === route ? "page" : undefined}>
            {r.id === route && (
              <motion.span
                className="pill steel"
                layoutId={layoutId}
                transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 500, damping: 40 }}
              />
            )}
            <Icon name={r.id} />
            <span className="label">{r.title}</span>
          </a>
        </li>
      ))}
    </ul>
  );
}
