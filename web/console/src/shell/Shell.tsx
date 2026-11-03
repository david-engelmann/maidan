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
import { liveWord, shellTitle } from "../core/firstRun.ts";
import { Chooser } from "./Chooser.tsx";
import { Icon, Mark } from "./icons.tsx";
import { SignIn } from "./SignIn.tsx";
import { Toasts } from "./Toasts.tsx";
import { Palette } from "./Palette.tsx";
import { LIST_ROWS, goSequence, isTyping, listStep, requestCompose, routeCommands, type Command } from "./commands.ts";
import { ROUTES, navigate, phoneTabs, usePlace, useRoute, type Route, type RouteId } from "./router.ts";
import { EmptyState } from "./EmptyState.tsx";

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
  const { session, store } = useSession();
  const signedIn = session.status === "signed-in";
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
    let seq: { at: number } | null = null;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
        return;
      }
      if (e.metaKey || e.ctrlKey || e.altKey || e.repeat || isTyping(e.target) || document.querySelector("[aria-modal='true']")) {
        seq = null;
        return;
      }
      if (!seq && (e.key === "j" || e.key === "k")) {
        // j and k walk the rows of the list on screen, in reading order.
        const rows = [...document.querySelectorAll<HTMLElement>(LIST_ROWS)].filter((r) => r.offsetParent !== null);
        const at = listStep(rows.findIndex((r) => r === document.activeElement || r.contains(document.activeElement)), rows.length, e.key);
        if (at !== null) {
          e.preventDefault();
          rows[at].focus();
          rows[at].scrollIntoView({ block: "nearest" });
        }
        return;
      }
      const next = goSequence(seq, e.key, e.timeStamp);
      seq = next.state;
      if (next.go) navigate(next.go);
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
      ...routeCommands(navigate),
      ...(signedIn
        ? ([
            { id: "new-message", title: "New message", hint: "Start a conversation with a person or agent", group: "Actions", run: () => { requestCompose(); navigate("threads"); } },
            { id: "upload", title: "Upload a file", hint: "Files: add an artifact and link it to a task", group: "Actions", run: () => navigate("artifacts") },
            { id: "refresh", title: "Refresh this screen", hint: "Read everything on this screen again", group: "Actions", run: () => liveBus.emit({ type: "resync" }) },
            { id: "sign-out", title: "Sign out", hint: "Forget this page's sign-in", group: "Actions", run: () => void store.logout() },
          ] satisfies Command[])
        : []),
    ],
    [signedIn, store],
  );

  const current = route === "not-found" ? NOT_FOUND : (ROUTES.find((r) => r.id === route) ?? ROUTES[0]);
  useEffect(() => {
    document.title = signedIn ? `${current.title} · Maidan` : "Sign in · Maidan";
  }, [current.title, signedIn]);
  const who = session.status === "signed-in" ? (session.displayName ?? session.memberId) : null;
  // Signed out, every place is the sign-in card: no place tabs to wander
  // through before the token field, and no "Offline" for a server that is up.
  const signedOut = session.status === "signed-out";
  const word = liveWord(live.kind, signedOut);

  return (
    <div className="shell">
      {/* Routes live in the hash, so the skip link moves focus itself rather than
          setting #main, which would read as a route. */}
      <a
        className="skip-link"
        href="#main"
        onClick={(e) => {
          e.preventDefault();
          document.getElementById("main")?.focus();
        }}
      >
        Skip to content
      </a>
      <nav className="rail" aria-label="Places">
        <a className="brand" href="#/board" aria-label="Maidan console">
          <Mark className="mark" />
          <span>Maidan</span>
        </a>
        {signedOut ? null : <Tabs route={route} reduce={Boolean(reduce)} layoutId="rail-pill" />}
      </nav>

      <header className={scrolled ? "top scrolled" : "top"}>
        <h1>{shellTitle(signedOut, current.title)}</h1>
        <div className="top-actions">
          {session.status === "signed-in" && session.mode && session.mode !== "session" ? (
            <span className="mode-chip" data-mode={session.mode} title="What this page acts with">{MODE_WORD[session.mode]}</span>
          ) : null}
          {live.kind !== "connected" ? (
            <span className={`live live-${live.kind}`} role="status" title={`${word}${who ? ` · signed in as ${who}` : ""}`}>
              <span className="dot" aria-hidden="true" />
              <span className="visually-hidden">{word}</span>
            </span>
          ) : null}
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
          aria-label={shellTitle(signedOut, current.title)}
        >
          {session.status === "signed-out" ? <SignIn noWorkspace={hint === "no-workspace"} /> : route === "not-found" ? <NotFound /> : screens[route]}
        </motion.section>
      </main>

      {signedOut ? null : (
        <nav className="tabbar" aria-label="Places">
          <PhoneTabs route={route} reduce={Boolean(reduce)} />
        </nav>
      )}

      {hint === "choose" && session.status === "signed-in" && !session.credentials.pastedToken && !session.credentials.tokenSession ? (
        <Chooser onClose={() => setHint(null)} />
      ) : null}
      <Toasts />
      <Palette open={paletteOpen} onClose={() => setPaletteOpen(false)} commands={commands} />
    </div>
  );
}

const NOT_FOUND = { title: "Not found" };

function NotFound() {
  const place = usePlace();
  return (
    <EmptyState
      title="Nothing here"
      body={`The console has no place called “${place}”. The link may be old or mistyped.`}
      action={<a className="button ghost" href="#/board">Go to the board</a>}
    />
  );
}

// Five places and a More that opens the rest, so labels stay readable at 390px.
function PhoneTabs({ route, reduce }: { route: Route; reduce: boolean }) {
  const { tabs, more } = phoneTabs();
  const inMore = more.some((r) => r.id === route);
  return (
    <div className="tabbar-row">
      <Tabs route={route} reduce={reduce} layoutId="tab-pill" routes={tabs} />
      <details className="tab-more" key={route}>
        <summary aria-current={inMore ? "page" : undefined}>
          <Icon name="more" />
          <span className="label">More</span>
        </summary>
        <ul className="tab-more-list">
          {more.map((r) => (
            <li key={r.id}>
              <a href={`#/${r.id}`} aria-current={r.id === route ? "page" : undefined}>
                <Icon name={r.id} />
                <span>{r.title}</span>
              </a>
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}

function Tabs({ route, reduce, layoutId, routes = ROUTES }: { route: Route; reduce: boolean; layoutId: string; routes?: readonly (typeof ROUTES)[number][] }) {
  return (
    <ul className="tabs">
      {routes.map((r) => (
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
