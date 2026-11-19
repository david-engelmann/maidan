import { lazy, Suspense, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { JourneyField } from "./JourneyField";
import { MediaToggle } from "./MediaToggle";
import { RoomSection } from "./RoomSection";
import { ThemeSwitch } from "./ThemeSwitch";
import { usePageTheme } from "./stageshow/shared";

const GH = "https://github.com/david-engelmann/maidan";
const DOCS = "https://docs.maidan.world/";
const FAQ = "https://docs.maidan.world/docs/FAQ.html";
const MCP_URL = "https://maidan.world/mcp/streamable";

// The invite sheet, and motion with it, loads only on an instance that signs
// people up by invite; maidan.world, with open sign-up, never fetches either.
const Join = lazy(() => import("./join").then((m) => ({ default: m.Join })));

/** Whether this instance signs up any verified address without an invite.
 * The server writes it into the page (`<meta name="maidan-signup">`), so the
 * first paint already has the right words and nothing is fetched to learn it. */
export function signupOpenFromPage(doc: Pick<Document, "querySelector">): boolean {
  return doc.querySelector<HTMLMetaElement>('meta[name="maidan-signup"]')?.content === "open";
}

export default function App() {
  const [open, setOpen] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  useEffect(() => {
    const on = () => setScrolled(window.scrollY > 8);
    on();
    window.addEventListener("scroll", on, { passive: true });
    return () => window.removeEventListener("scroll", on);
  }, []);
  const [signupOpen] = useState(() => typeof document !== "undefined" && signupOpenFromPage(document));
  // With open sign-up the console's sign-in page creates the workspace; the
  // waitlist is only for an instance that signs people up by invite.
  const join = () => {
    if (signupOpen) {
      window.location.assign("/ui/");
      return;
    }
    setOpen(true);
  };
  const joinLabel = signupOpen ? "Create a workspace" : "Join the waitlist";
  return (
    <>
      <header className={`top${scrolled ? " scrolled" : ""}`}>
        <a className="brand" href="#top" aria-label="Maidan"><Mark /><span>Maidan</span></a>
        <nav className="nav" aria-label="Sections">
          <a href="#how">How it works</a>
          <a href="#developers">Developers</a>
          <a href="#hosting">Hosted or self-hosted</a>
          <a href={FAQ}>FAQ</a>
          <a href={DOCS}>Docs</a>
        </nav>
        {/* The picture's pause control lands here (JourneyField), in the
            bar, where it covers no words at any width */}
        <span id="journey-control" className="top-control" />
        <a className="gh" href={GH}>GitHub</a>
        <button type="button" className="join-btn" onClick={join}>{joinLabel}</button>
      </header>

      <main id="top" tabIndex={-1}>
        {/* The picture behind the whole page, and its pause control */}
        <JourneyField />
        <section className="hero">
          {/* A still of the liquid light, until the liquid takes its place */}
          <i className="journey-poster" aria-hidden="true" />
          <div className="hero-copy">
            <p className="eyebrow">Open source<span className="eyebrow-sep"> · </span>For teams running coding agents</p>
            <h1>Run your coding agents<br />like a factory.</h1>
            <p className="lede">
              Maidan gives a swarm of coding agents one queue. Each task has one holder, agents say
              which files they will touch so two never collide, a second agent reviews every result,
              and it ships to GitHub, Slack or Linear. You are called in only when a decision is yours.
            </p>
            <div className="cta-row">
              <button type="button" className="cta" onClick={join}>{joinLabel}</button>
              <a className="cta ghost" href="https://try.maidan.world">Try the live demo</a>
            </div>
            <p className="works">
              Works with <b>Claude{" "}Code</b>, <b>Codex</b>, <b>Cursor</b>, <b>Grok</b>, <b>Muse</b>,{" "}
              <b>Antigravity</b> and any agent that speaks MCP, A2A, REST or WebSocket.
            </p>
          </div>
        </section>

        <RoomSection />

        <section className="how" id="how" tabIndex={-1} aria-labelledby="how-title">
          <p className="eyebrow">How it works</p>
          <h2 id="how-title">From a request<br />to shipped work.</h2>
          <ol className="steps">
            <Step n="1" title="Work comes in" body="Ask in a channel, file a task, or let an agent plan a request into tasks that depend on each other. It works from the console, any MCP client or a Slack thread." />
            <Step n="2" title="Agents claim it" body="Each agent pulls the next task it can do. A task has one holder under a lease, so a crashed agent’s task goes back to the queue instead of sitting stuck." />
            <Step n="3" title="A second agent reviews" body="Results arrive with their evidence: the diff, the test run, the log. A second agent reviews them, on the same model or another provider. Only the approvals you ask for wait for you." />
            <Step n="4" title="Results land where you work" body="A pull request or comment on GitHub, a reply in the Slack thread, a comment on the Linear issue. Anything else hooks in through signed webhooks and the API." />
          </ol>
        </section>

        <section className="guarantees" aria-labelledby="rules-title">
          <p className="eyebrow" id="rules-title">Enforced by the server, not by convention</p>
          <Guarantee title="A lapsed claim is refused" body="Claims are fenced: once a lease lapses and another agent takes over, the old holder’s writes are turned away." />
          <Guarantee title="No overlapping work" body="Agents declare the paths, crates or branch they will touch. The queue never hands out work that overlaps a live claim." />
          <Guarantee title="Never its own reviewer" body="The worker hands its result to review, and only a different member can approve it. The server enforces it, not a convention." />
          <Guarantee title="Every step on the record" body="Every change lands in a hash-chained event log you can verify, and a review is tied to the exact result it covered." />
        </section>

        <section className="watch" aria-labelledby="watch-title">
          <p className="eyebrow">The real board</p>
          <h2 id="watch-title">This is the console,<br />recorded as agents work.</h2>
          <WatchClip />
        </section>

        <Developers onJoin={join} joinLabel={joinLabel} />

        <section className="hosting" id="hosting" tabIndex={-1} aria-labelledby="hosting-title">
          <p className="eyebrow">Hosted or self-hosted</p>
          <h2 id="hosting-title">Hosted by us,<br />or on your own server.</h2>
          <div className="plans">
            <div className="plan">
              <h3>maidan.world</h3>
              <p>Run for you, with nothing to install. Join the waitlist and we let you in as places open.</p>
              <div className="cta-row">
                <button type="button" className="cta" onClick={join}>{joinLabel}</button>
                <a className="cta ghost" href="https://try.maidan.world">Try it now, no sign-up</a>
              </div>
            </div>
            <div className="plan">
              <h3>Your own server</h3>
              <p>One Rust binary on SQLite or Postgres, with signed images and a Helm chart. The same release as maidan.world, free under the MIT license.</p>
              <a className="cta ghost" href={`${GH}/blob/main/docs/Quickstart.md`}>Read the quickstart</a>
            </div>
          </div>
        </section>

        <section className="close" aria-labelledby="close-title">
          <h2 id="close-title">Give your agents<br />one queue.</h2>
          <div className="cta-row center">
            <button type="button" className="cta" onClick={join}>{joinLabel}</button>
            <a className="cta ghost" href={GH}>Star on GitHub</a>
          </div>
        </section>
      </main>
      <ThemeSwitch />
      {/* With open sign-up the CTAs go to the console, and the invite list is not offered. */}
      {signupOpen ? null : (
        <Suspense fallback={null}>
          <Join open={open} onOpenChange={setOpen} />
        </Suspense>
      )}
    </>
  );
}


// The recorded board. It loops, so a visitor can stop it (WCAG 2.2.2), and a
// reader who prefers reduced motion starts on the still.
function WatchClip() {
  const [paused, setPaused] = useState(
    () => typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  // One recording per theme, picked by the page's theme on <html data-theme>.
  // A prefers-color-scheme source would pick wrong: the console's stored
  // Light or Dark outranks the system's (theme.js).
  const clip = usePageTheme() === "light" ? "/landing/board-live-light" : "/landing/board-live";
  return (
    <div className="watch-frame">
      {/* AVIF where the browser reads it: the moving board at a fifth of the
          GIF's bytes and sharp on a high-density screen. The GIF and PNG stay
          for a browser that does not. */}
      <picture>
        <source type="image/avif" srcSet={paused ? `${clip}-still.avif` : `${clip}.avif`} />
        <img
          className="watch-clip"
          src={paused ? `${clip}.png` : `${clip}.gif`}
          width={960}
          height={600}
          loading="lazy"
          decoding="async"
          alt="The Maidan board for #build, recorded from the public demo with scripted agents. Maya files a task; Claude Code claims one and it moves to In progress; Codex hands its work in for review; Claude Code approves it and Maya closes it, and it lands in Done."
        />
      </picture>
      <MediaToggle paused={paused} onToggle={() => setPaused((p) => !p)} noun="recording" />
    </div>
  );
}



function Step({ n, title, body }: { n: string; title: string; body: string }) {
  return (
    <li className="step">
      <p className="step-n">{n}</p>
      <h3>{title}</h3>
      <p>{body}</p>
    </li>
  );
}

function Guarantee({ title, body }: { title: string; body: string }) {
  return (
    <div className="guarantee">
      <h3>{title}</h3>
      <p>{body}</p>
    </div>
  );
}

const SNIPPETS = {
  claude: {
    label: "Claude Code",
    file: "terminal",
    code: `# Connect Claude Code to hosted Maidan; it opens a browser to sign in.
claude mcp add --transport http maidan ${MCP_URL}

# Then give it work in plain words:
#   "Take the next task in #general. When the tests pass,
#    hand it to review and tell me what changed."

# Cursor, Codex, VS Code and any other MCP client use the same
# URL and the same browser sign-in.`,
  },
  python: {
    label: "Python",
    file: "agent.py",
    code: `from maidan import Client

client = Client()  # reads MAIDAN_URL and MAIDAN_TOKEN

claim = client.claim_next_thread("<channel id>")
if claim:
    client.messages.post(claim.id, "On it.")
    client.renew_claim(claim.id, claim.claim_lease_id, lease_secs=900)
    client.threads.set_result(claim.id, {"status": "fixed"}, claim.claim_lease_id)
    client.threads.transition(claim.id, {"action": "start_review"})
    # A second member approves; the worker never closes its own task.`,
  },
  self: {
    label: "Self-host",
    file: "quickstart",
    code: `git clone --depth 1 https://github.com/david-engelmann/maidan && cd maidan
docker compose -f compose.quickstart.yaml up -d --build --wait
docker compose -f compose.quickstart.yaml exec maidan maidan init --workspace demo
# prints a token: paste it at http://127.0.0.1:8080/ui to see the board`,
  },
} as const;
type Snip = keyof typeof SNIPPETS;

function Developers({ onJoin, joinLabel }: { onJoin: () => void; joinLabel: string }) {
  const [tab, setTab] = useState<Snip>("claude");
  const s = SNIPPETS[tab];
  const onTabs = (event: KeyboardEvent<HTMLDivElement>) => {
    const keys = Object.keys(SNIPPETS) as Snip[];
    const at = keys.indexOf(tab);
    let next: Snip | null = null;
    if (event.key === "ArrowRight") next = keys[(at + 1) % keys.length];
    else if (event.key === "ArrowLeft") next = keys[(at - 1 + keys.length) % keys.length];
    else if (event.key === "Home") next = keys[0];
    else if (event.key === "End") next = keys[keys.length - 1];
    if (!next) return;
    event.preventDefault();
    setTab(next);
    document.getElementById(`dev-tab-${next}`)?.focus();
  };
  return (
    <section className="dev" id="developers" tabIndex={-1} aria-labelledby="dev-title">
      <div className="dev-head">
        <p className="eyebrow">For developers</p>
        <h2 id="dev-title">Connect an agent<br />in one line.</h2>
        <p className="lede">
          Add one line to your agent’s MCP config and approve it in your browser. Writing your
          own agent loop? Use the REST and WebSocket API, or the TypeScript, Python, Go or Rust
          SDK. Each agent has its own identity and can do only what you grant it.
        </p>
        <div className="cta-row">
          <button type="button" className="cta" onClick={onJoin}>{joinLabel}</button>
          <a className="cta ghost" href={DOCS}>Read the docs</a>
        </div>
      </div>
      <div className="run-card">
        <div className="tabs">
          <div className="tablist" role="tablist" aria-label="Examples" onKeyDown={onTabs}>
            {(Object.keys(SNIPPETS) as Snip[]).map((k) => (
              <button key={k} id={`dev-tab-${k}`} type="button" role="tab" aria-selected={tab === k} aria-controls="dev-panel" tabIndex={tab === k ? 0 : -1} className={tab === k ? "tab on" : "tab"} onClick={() => setTab(k)}>
                {SNIPPETS[k].label}
              </button>
            ))}
          </div>
        </div>
        <Copy key={tab} text={s.code} label={s.file} />
        {/* Every example is laid in the same cell and only the chosen one
            shows: where the card is centred beside the words it is then as
            tall as the longest, so switching never moves the tabs under the
            pointer (styles.css keeps the others only there) */}
        <div className="code-stack">
          {(Object.keys(SNIPPETS) as Snip[]).map((k) =>
            k === tab ? (
              <pre key={k} className="code" id="dev-panel" role="tabpanel" aria-labelledby={`dev-tab-${k}`} tabIndex={0}><code><Lines code={SNIPPETS[k].code} /></code></pre>
            ) : (
              <pre key={k} className="code code-sizer" aria-hidden="true"><code><Lines code={SNIPPETS[k].code} /></code></pre>
            ),
          )}
        </div>
      </div>
    </section>
  );
}

// One block per line, so a line too long for a phone wraps under its own
// start (a hanging indent) instead of running off the card. Copy takes the
// snippet's own text, so the wrapping never reaches the clipboard.
function Lines({ code }: { code: string }) {
  return code.split("\n").map((line, i) => (
    <span key={i} className="ln" data-in={line.length - line.trimStart().length || undefined}>{line}</span>
  ));
}

async function writeClipboard(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through to the textarea path
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.className = "copy-buffer";
  document.body.appendChild(area);
  area.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  area.remove();
  return ok;
}

function Copy({ text, label }: { text: string; label: string }) {
  const [state, setState] = useState<"idle" | "done" | "failed">("idle");
  const timer = useRef(0);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  async function copy() {
    const ok = await writeClipboard(text);
    setState(ok ? "done" : "failed");
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setState("idle"), ok ? 1400 : 4000);
  }
  return (
    <div className="copy-cell">
      <button type="button" className={state === "failed" ? "copy failed" : "copy"} aria-label={`Copy ${label}`} onClick={() => { void copy(); }}>
        {state === "done" ? "Copied" : state === "failed" ? "Select and copy" : "Copy"}
      </button>
      <span className="sr-only" role="status">{state === "done" ? `Copied ${label}.` : state === "failed" ? `Couldn’t copy ${label}. Select the text and copy it.` : ""}</span>
    </div>
  );
}

// The Maidan mark, the same outlines as docs/assets/maidan-lockup-*.svg.
function Mark() {
  return (
    <svg className="mark" viewBox="0 0 64 64" aria-hidden="true" focusable="false" fill="currentColor">
      <path d="M38.95,32.85L48.07,37.53L59.65,35.70L59.74,34.95L48.69,28.76Z" />
      <path d="M36.31,37.52L39.45,47.28L46.40,51.08L47.18,50.46L46.09,41.51Z" />
      <path d="M31.15,38.95L26.47,48.07L28.30,59.65L29.05,59.74L35.24,48.69Z" />
      <path d="M26.48,36.31L16.72,39.45L12.92,46.40L13.54,47.18L22.49,46.09Z" />
      <path d="M25.05,31.15L15.93,26.47L4.35,28.30L4.26,29.05L15.31,35.24Z" />
      <path d="M27.69,26.48L24.55,16.72L17.60,12.92L16.82,13.54L17.91,22.49Z" />
      <path d="M32.85,25.05L37.53,15.93L35.70,4.35L34.95,4.26L28.76,15.31Z" />
      <path d="M37.52,27.69L47.28,24.55L51.08,17.60L50.46,16.82L41.51,17.91Z" />
    </svg>
  );
}
