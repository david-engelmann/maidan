// The app shell, iPhone first: a glass top bar under the status bar, a tab
// bar over the home indicator, and the same four places as a rail on a wide
// screen. Safe-area insets come from viewport-fit=cover in index.html.
import { motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { controlBus, liveBus, liveStatus } from "../core/bus.ts";
import type { ControlFrame, EventFrame } from "../core/live.ts";
import { useLive } from "../hooks/useLive.ts";
import { useSession } from "../hooks/useSession.ts";
import { MODE_WORD } from "../core/api.ts";
import { readFrontDoorHint } from "../core/session.ts";
import { Chooser } from "./Chooser.tsx";
import { Icon, Mark } from "./icons.tsx";
import { SignIn } from "./SignIn.tsx";
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
  // A refused socket (a revoked or narrowed token) sends no more events, so
  // ask every screen to read once more: each then shows the refusal it gets.
  useEffect(() => liveStatus.set(live), [live]);
  const refused = live.kind === "refused";
  useEffect(() => {
    if (refused) liveBus.emit({ type: "resync" });
  }, [refused]);
  const reduce = useReducedMotion();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  // The front door's hint is read once and taken off the address bar, so a
  // reload never replays it.
  const [hint, setHint] = useState(() => readFrontDoorHint(window.location.search).hint);
  useEffect(() => {
    const { rest } = readFrontDoorHint(window.location.search);
    if (rest !== window.location.search) window.history.replaceState(null, "", `${window.location.pathname}${rest}${window.location.hash}`);
  }, []);

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
    ],
    [],
  );

  const current = ROUTES.find((r) => r.id === route) ?? ROUTES[0];
  const who = session.status === "signed-in" ? (session.displayName ?? session.memberId) : null;
  const liveWord =
    live.kind === "connected" ? "Live" : live.kind === "reconnecting" ? "Reconnecting" : live.kind === "refused" ? "Refused" : "Offline";

  return (
    <div className="shell">
      <a className="skip-link" href="#main" onClick={(e) => { e.preventDefault(); document.getElementById("main")?.focus(); }}>Skip to content</a>
      <nav className="rail" aria-label="Places">
        <a className="brand" href="#/board" aria-label="Maidan console">
          <Mark className="mark" />
          <span>Maidan</span>
        </a>
        <Tabs route={route} reduce={Boolean(reduce)} layoutId="rail-pill" />
      </nav>

      <header className={scrolled ? "top scrolled" : "top"}>
        <h1>{current.title}</h1>
        <div className="top-actions">
          {session.status === "signed-in" && session.mode ? (
            <span className="mode-chip" data-mode={session.mode} title="What this page acts with">{MODE_WORD[session.mode]}</span>
          ) : null}
          <span className={`live live-${live.kind}`} role="status" title={who ? `Signed in as ${who}` : "Not signed in"}>
            <span className="dot" aria-hidden="true" />
            {liveWord}
          </span>
          <button className="icon-btn" aria-label="Search and commands (⌘K)" onClick={() => setPaletteOpen(true)}>
            <Icon name="search" />
          </button>
        </div>
      </header>

      <main className="content" id="main" tabIndex={-1}>
        <motion.section
          key={route}
          initial={reduce ? false : { opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.2, ease: [0.2, 0.7, 0.2, 1] }}
          aria-label={current.title}
        >
          {session.status === "signed-out" ? <SignIn noWorkspace={hint === "no-workspace"} /> : screens[route]}
        </motion.section>
      </main>

      <nav className="tabbar" aria-label="Places">
        <Tabs route={route} reduce={Boolean(reduce)} layoutId="tab-pill" />
      </nav>

      {hint === "choose" && session.status === "signed-in" && !session.credentials.pastedToken && !session.credentials.tokenSession ? (
        <Chooser onClose={() => setHint(null)} />
      ) : null}
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
