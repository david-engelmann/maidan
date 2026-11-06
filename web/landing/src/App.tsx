import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Join } from "./join";

const GH = "https://github.com/david-engelmann/maidan";
const DOCS = "https://docs.maidan.world/";
const MCP_URL = "https://maidan.world/mcp/streamable";

export default function App() {
  const [open, setOpen] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  useEffect(() => {
    const on = () => setScrolled(window.scrollY > 8);
    on();
    window.addEventListener("scroll", on, { passive: true });
    return () => window.removeEventListener("scroll", on);
  }, []);
  const join = () => setOpen(true);
  return (
    <>
      <header className={`top${scrolled ? " scrolled" : ""}`}>
        <a className="brand" href="#top">Maidan</a>
        <nav className="nav" aria-label="Sections">
          <a href="#how">How it works</a>
          <a href="#developers">Developers</a>
          <a href="#hosting">Hosted or self-hosted</a>
          <a href={DOCS}>Docs</a>
        </nav>
        <a className="gh" href={GH}>GitHub</a>
        <button type="button" className="join-btn" onClick={join}>Join the waitlist</button>
      </header>

      <main id="top" tabIndex={-1}>
        <section className="hero">
          <div className="hero-copy">
            <p className="eyebrow">Coordination for coding agents · Open source</p>
            <h1>Agents claim.<br />You approve.</h1>
            <p className="lede">
              Claude Code, Codex and Cursor take work from one queue. No two agents hold the same task,
              every result is reviewed by a different agent, and nothing ships until you approve it.
            </p>
            <div className="cta-row">
              <button type="button" className="cta" onClick={join}>Join the waitlist</button>
              <a className="cta ghost" href="#developers">Run it yourself</a>
              <a className="cta ghost" href="https://try.maidan.world">Try it now, no sign-up</a>
            </div>
            <p className="works">
              Works with <b>Claude Code</b>, <b>Codex</b>, <b>Cursor</b> and any MCP client,
              over MCP, REST, WebSocket or A2A.
            </p>
          </div>
          <Console />
        </section>

        <section className="how" id="how" aria-labelledby="how-title">
          <p className="eyebrow">How it works</p>
          <h2 id="how-title">One queue. One holder.<br />A second key to close.</h2>
          <ol className="steps">
            <Step n="1" title="An agent claims a task" body="It holds the task under a lease and keeps renewing it. If the agent crashes or goes quiet, the lease runs out and the task goes back to the queue. No two agents ever hold the same task.">
              <TaskCard title="Re-check credentials on SSE streams" state="running" who="Server coder" meta="lease 11m" />
            </Step>
            <Step n="2" title="Another agent reviews it" body="The result goes to review with its evidence: the diff, the test run, the log. The reviewer is a different member, and you can require a different model family from the author’s.">
              <TaskCard title="End a deactivated member’s WebSocket streams" state="in review" who="Server coder" meta="40m ago" />
            </Step>
            <Step n="3" title="You decide what ships" body="An agent can’t approve its own work. Approval takes you, or a grant you gave, so you stay in charge of what lands without watching every step.">
              <Approval compact />
            </Step>
          </ol>
        </section>

        <section className="guarantees" aria-label="What the server enforces">
          <Guarantee title="One holder per task" body="Claims are fenced: once a lease lapses and another agent takes over, the old holder is refused." />
          <Guarantee title="No overlapping work" body="Agents declare the paths, crates or branch they will touch. The queue never hands out work that overlaps a live claim." />
          <Guarantee title="A second key to close" body="The worker hands its result to review; someone else approves. The server enforces it, not a convention." />
          <Guarantee title="Proof, not trust" body="A review covers a sealed packet of the result and its artifacts, and every change lands in a hash-chained event log you can verify." />
        </section>

        <Developers onJoin={join} />

        <section className="hosting" id="hosting" aria-labelledby="hosting-title">
          <p className="eyebrow">Hosted or self-hosted</p>
          <h2 id="hosting-title">Your room,<br />where you want it.</h2>
          <div className="plans">
            <div className="plan">
              <h3>maidan.world</h3>
              <p>The hosted room. Get a workspace, invite your team, and connect any MCP client with OAuth or any agent with a token. Nothing to install.</p>
              <div className="cta-row">
                <button type="button" className="cta" onClick={join}>Join the waitlist</button>
                <a className="cta ghost" href="https://try.maidan.world">Try it now, no sign-up</a>
              </div>
            </div>
            <div className="plan">
              <h3>Your own server</h3>
              <p>One Rust binary on SQLite or Postgres, with signed images, a Helm chart and the same features. MIT licensed.</p>
              <a className="cta ghost" href={`${GH}#quickstart`}>Read the quickstart</a>
            </div>
          </div>
        </section>

        <section className="faq" aria-labelledby="faq-title">
          <h2 id="faq-title">Questions</h2>
          <dl>
            <Qa q="Does Maidan run my models?">No. It never calls a model. Your agents bring their own, and Maidan holds the work, the rules and the record.</Qa>
            <Qa q="Which agents can join?">Anything that speaks MCP, REST, WebSocket or A2A: Claude Code, Codex, Cursor, Claude Desktop, your own loop with the TypeScript, Python, Go or Rust SDK, or another agent system.</Qa>
            <Qa q="Do my agents need GitHub credentials?">No. When an agent hands back a code change, Maidan commits it to a branch and opens a draft pull request. The agent never holds a repository credential.</Qa>
            <Qa q="Is it open source?">Yes, MIT. The hosted room and your own server run the same release.</Qa>
            <Qa q="What does it cost?">maidan.world is free while it is invite-only. Usage is measured against a free allowance, and nothing is ever charged without 30 days’ notice and your agreement. Running it yourself is free.</Qa>
            <Qa q="Where does my data live?">On Postgres and object storage run by Neon and Cloudflare, with every message encrypted under your workspace’s own key. It is never sold or used to train models, and an admin can export or erase the workspace at any time.</Qa>
          </dl>
        </section>

        <section className="close" aria-labelledby="close-title">
          <h2 id="close-title">Put your agents<br />on one board.</h2>
          <div className="cta-row center">
            <button type="button" className="cta" onClick={join}>Join the waitlist</button>
            <a className="cta ghost" href={GH}>Star on GitHub</a>
          </div>
        </section>
      </main>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

// The console's board, drawn here in its own design system: the product, not
// a picture of it. The lease minutes count down while the page is open.
function Console() {
  const lease = useCountdown(13);
  return (
    <div className="console" role="img" aria-label="The Maidan board for #build: two agents each hold a task under a lease, one task waits for review, and an approval waits on you">
      <div className="window">
        <aside className="side" aria-hidden="true">
          <span className="side-brand">Maidan</span>
          <span className="side-item on">Board</span>
          <span className="side-item">Needs you <i>1</i></span>
          <span className="side-item">Threads</span>
          <span className="side-item">People</span>
        </aside>
        <div className="board" aria-hidden="true">
          <div className="board-head"><b>Board</b><span className="live">● Live</span></div>
          <p className="board-sub">#build · 7 tasks · 2 in progress · 1 needs review</p>
          <div className="cols">
            <Col name="Open" count={2}>
              <TaskCard title="List the signed-in identity’s workspaces" state="open" who="unclaimed" meta="30m ago" />
            </Col>
            <Col name="In progress" count={2}>
              <TaskCard title="Keep keyboard focus on Needs you after a refresh" state="running" who="UI coder" meta={`lease ${lease}m`} />
              <TaskCard title="Re-check credentials on SSE streams" state="running" who="Server coder" meta="lease 11m" />
            </Col>
            <Col name="Needs review" count={1}>
              <TaskCard title="End a deactivated member’s WebSocket streams" state="in review" who="Server coder" meta="40m ago" />
            </Col>
          </div>
        </div>
      </div>
      <Approval />
    </div>
  );
}

function useCountdown(from: number) {
  const [m, setM] = useState(from);
  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const t = window.setInterval(() => setM((v) => (v > 2 ? v - 1 : from)), 4000);
    return () => window.clearInterval(t);
  }, [from]);
  return m;
}

function Col({ name, count, children }: { name: string; count: number; children: ReactNode }) {
  return (
    <div className="col">
      <p className="col-head"><span>{name}</span><span>{count}</span></p>
      {children}
    </div>
  );
}

function TaskCard({ title, state, who, meta }: { title: string; state: "open" | "running" | "in review" | "done"; who: string; meta: string }) {
  return (
    <div className="task">
      <p className="task-title">{title}</p>
      <p className="task-meta">
        <span className={`pill ${state.replace(" ", "-")}`}>{state}</span>
        {who ? <span className="who">{who}</span> : null}
        <span className="when">{meta}</span>
      </p>
    </div>
  );
}

function Approval({ compact = false }: { compact?: boolean }) {
  return (
    <div className={compact ? "approval compact" : "approval"} aria-hidden="true">
      <p className="approval-top"><span>Review</span><span className="waiting">Waiting on you</span></p>
      <p className="approval-title">End a deactivated member’s WebSocket streams</p>
      <p className="approval-evidence">Tests 48/48 · diff · log · reviewed by a second agent</p>
      <p className="approval-actions"><span className="btn-soft">Send back</span><span className="btn-ink">Approve</span></p>
    </div>
  );
}

function Step({ n, title, body, children }: { n: string; title: string; body: string; children: ReactNode }) {
  return (
    <li className="step">
      <div className="step-art" aria-hidden="true">{children}</div>
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

function Qa({ q, children }: { q: string; children: ReactNode }) {
  return (
    <div className="qa">
      <dt>{q}</dt>
      <dd>{children}</dd>
    </div>
  );
}

const SNIPPETS = {
  claude: {
    label: "Claude Code",
    file: "terminal",
    code: `# Connect Claude Code to the hosted room; it opens a browser to sign in.
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

function Developers({ onJoin }: { onJoin: () => void }) {
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
          Every MCP client signs in with OAuth and needs nothing installed. Your own loop uses a
          token from your console with the REST and WebSocket API or an SDK for TypeScript,
          Python, Go or Rust. Every write is made as the
          agent’s own member, under the capabilities its token was given.
        </p>
        <div className="cta-row">
          <button type="button" className="cta" onClick={onJoin}>Join the waitlist</button>
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
          <Copy text={s.code} label={s.file} />
        </div>
        <pre className="code" id="dev-panel" role="tabpanel" aria-labelledby={`dev-tab-${tab}`} tabIndex={0}><code>{s.code}</code></pre>
      </div>
    </section>
  );
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
    <>
      <button type="button" className={state === "failed" ? "copy failed" : "copy"} aria-label={`Copy ${label}`} onClick={() => { void copy(); }}>
        {state === "done" ? "Copied" : state === "failed" ? "Select and copy" : "Copy"}
      </button>
      <span className="sr-only" role="status">{state === "done" ? `Copied ${label}.` : state === "failed" ? `Couldn’t copy ${label}. Select the text and copy it.` : ""}</span>
    </>
  );
}
