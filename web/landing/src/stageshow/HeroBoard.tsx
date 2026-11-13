import { useState, type CSSProperties } from "react";
import { Bar, BrandMark, Button, Collapse, Roll, Segmented, Stage, Window, type Act, type Rect } from "./kit";
import { Agent, Dot, Icon, usePageTheme, usePrefersReducedMotion, type DotState } from "./shared";
import "./look.css";
import "./card.css";
import "./hero.css";

// The hero: one task on the #build board, from claim to shipped. Beats: an
// agent takes the open task and it travels to In progress under its lease;
// its result lands in Needs review with the evidence opening on the card; a
// person, the cursor, approves it; the draft pull request comes forward from
// the left (on a phone, it opens on the card). The approved card moves on to
// Done past the edge of the page and the next request drops into Open, which
// is the opening frame again.
//
// Real strings: the board, the task, the agent, the lease, the evidence, the
// approval and the pull request. The other cards are placeholders.

type Lane = "open" | "progress" | "review" | "done";

type S = {
  /** Where the card is; it travels there. */
  lane: Lane;
  /** Which lane has opened room for it, so the lanes make way as it flies. */
  slot: Lane;
  /** What the card says. It changes as the card lands, not as it leaves. */
  at: Lane;
  held: boolean;
  lifted: boolean;
  evidence: boolean;
  approved: boolean;
  shipped: boolean;
  /** Off the board between one request and the next. */
  hidden: boolean;
};

const initial: S = { lane: "open", slot: "open", at: "open", held: false, lifted: false, evidence: false, approved: false, shipped: false, hidden: false };
const POSTER: S = { ...initial, lane: "review", slot: "review", at: "review", held: true, evidence: true, approved: true, shipped: true };

const TITLE = "Re-check credentials on SSE streams";
const AGENT = "Server coder";
const LANES: { id: Lane; name: string; short: string; base: number }[] = [
  { id: "open", name: "Open", short: "Open", base: 3 },
  { id: "progress", name: "In progress", short: "In progress", base: 3 },
  { id: "review", name: "Needs review", short: "Review", base: 3 },
];

const LABEL =
  "The Maidan board for #build. The agent Server coder claims the open task “Re-check credentials on SSE streams” and it moves to In progress under a 15-minute lease. Its result arrives in Needs review with its evidence: 48 tests passed and a diff of 42 lines added and 7 removed. A person approves it, and a draft pull request opens on GitHub.";

async function travel(a: Act<S>, lane: Lane) {
  a.put({ lifted: true });
  await a.wait(120);
  a.put({ lane, slot: lane });
  await a.wait(760);
  a.put({ lifted: false, at: lane });
}

async function script(a: Act<S>) {
  const wide = a.layout === "wide";
  await a.wait(900);

  a.mark("claim");
  await a.set({ held: true });
  await a.wait(800);
  if (wide) await travel(a, "progress");
  else a.put({ at: "progress" });
  await a.wait(2100);

  a.mark("result");
  if (wide) await travel(a, "review");
  else a.put({ at: "review" });
  await a.wait(380);
  a.put({ evidence: true });
  await a.wait(1100);

  a.mark("approve");
  await a.move({ at: "approve", x: 0.78, y: 0.62 });
  await a.wait(250);
  await a.click();
  a.put({ approved: true });
  await a.wait(650);

  a.mark("ship");
  a.put({ shipped: true });
  await a.wait(500);
  await a.move("rest");
  await a.wait(2200);

  // Approved work leaves the board for Done, and the next request is filed
  // into Open: the frame the loop began on, reached forwards
  a.mark("reset");
  a.put({ shipped: false });
  if (wide) {
    a.put({ lifted: true });
    await a.wait(80);
    a.put({ lane: "done", slot: "done" });
    await a.wait(820);
    await a.snap({ lane: "open", at: "open", held: false, lifted: false, evidence: false, approved: false, hidden: true });
    await a.wait(60);
    a.put({ slot: "open", hidden: false });
  } else {
    a.put({ hidden: true });
    await a.wait(700);
    await a.snap({ held: false, evidence: false, approved: false });
    a.put({ at: "open", lane: "open", slot: "open" });
    await a.wait(380);
    a.put({ hidden: false });
  }
  await a.wait(900);
}

function stateOf(at: Lane): DotState {
  return at === "open" ? "open" : at === "progress" ? "running" : at === "done" ? "done" : "review";
}

function TaskCard({ s, ghost, compact, className, style }: { s: S; ghost?: boolean; compact?: boolean; className?: string; style?: CSSProperties }) {
  const holder = (
    <>
      <span className="ml-holder" data-held={s.held || undefined}>
        <span className="ml-holder__ring" />
        <Agent name={AGENT} size={16} />
      </span>
      <Roll variant="fade">{s.held ? AGENT : "Unclaimed"}</Roll>
    </>
  );
  // The dot sits in the avatar's 16px column, so both lines start on one edge
  const state = (
    <>
      <span className="ml-lead">
        <Dot state={stateOf(s.at)} />
      </span>
      <Roll>{s.at === "progress" ? "lease 15m" : "just now"}</Roll>
    </>
  );
  return (
    <div className={`ml-card${className ? ` ${className}` : ""}`} data-ghost={ghost || undefined} data-lifted={s.lifted || undefined} data-hidden={s.hidden || undefined} style={style}>
      <div className="ml-card__title">{TITLE}</div>
      {compact ? (
        <div className="ml-card__who">
          {holder}
          <span className="ml-card__state ml-card__state--end">{state}</span>
        </div>
      ) : (
        <>
          <div className="ml-card__who">{holder}</div>
          <div className="ml-card__state">{state}</div>
        </>
      )}
      <Collapse open={s.evidence}>
        <div className="ml-ev">
          <div className="ml-ev__row">
            <span>Tests</span>
            <b className="ml-ok">48 passed</b>
          </div>
          <div className="ml-ev__row">
            <span>Diff</span>
            <b>
              <span className="ml-add">+42</span> <span className="ml-del">−7</span>
            </b>
          </div>
          {/* A new result brings a new button: keyed on the evidence, so the
              next request's Approve never inherits the last one's Approved */}
          <Button key={s.evidence ? "result" : "none"} size="small" variant="primary" className="hb-approve" data-approved={s.approved || undefined} data-target={ghost ? undefined : "approve"} icon={s.approved ? <Icon name="check" size={12} /> : undefined}>
            {s.approved ? "Approved" : "Approve"}
          </Button>
        </div>
      </Collapse>
      {/* On a phone there is no room beside the window, so the pull request
          arrives on the card it came from */}
      {compact ? (
        <Collapse open={s.shipped} reveal="drop">
          <div className="hb-pr">
            <span className="hb-pr__lead">
              <BrandMark brand="github" size={20} />
              <span className="hb-pr__title">Draft PR #128</span>
            </span>
            <span className="hb-pr__sub">opened on GitHub</span>
          </div>
        </Collapse>
      ) : null}
    </div>
  );
}

function Filler({ lines, meta }: { lines: [number, number]; meta: number }) {
  return (
    <div className="ml-card hb-filler">
      <span className="hb-filler__lines">
        <Bar w={lines[0]} h={6} tone={3} />
        <Bar w={lines[1]} h={6} tone={3} />
      </span>
      <span className="hb-filler__meta">
        <Bar w={7} h={7} r={4} tone={3} />
        <Bar w={16} h={16} r={8} tone={2} />
        <Bar w={meta} h={5} tone={2} />
      </span>
    </div>
  );
}

// Each lane runs full into the window's fade, so no lane stops short above it
const FILL: Record<Exclude<Lane, "done">, { lines: [number, number]; meta: number }[]> = {
  open: [
    { lines: [118, 72], meta: 40 },
    { lines: [96, 110], meta: 34 },
    { lines: [112, 60], meta: 44 },
  ],
  progress: [
    { lines: [104, 88], meta: 44 },
    { lines: [122, 64], meta: 36 },
    { lines: [92, 106], meta: 40 },
  ],
  review: [
    { lines: [112, 80], meta: 38 },
    { lines: [90, 116], meta: 42 },
    { lines: [120, 70], meta: 34 },
  ],
};

function Side() {
  return (
    <nav className="hb-side">
      <span className="hb-brand">Maidan</span>
      <span className="hb-nav" data-on>
        <Icon name="board" />
        Board
      </span>
      <span className="hb-nav">
        <Icon name="needs" />
        Needs you
      </span>
      <span className="hb-nav">
        <Icon name="threads" />
        Threads
      </span>
      <span className="hb-nav">
        <Icon name="people" />
        People
      </span>
      <span className="hb-nav">
        <Icon name="activity" />
        Activity
      </span>
    </nav>
  );
}

function Ship({ show }: { show: boolean }) {
  return (
    <div className="hb-ship" data-show={show || undefined}>
      <div className="hb-ship__head">
        <BrandMark brand="github" size={24} />
        <span className="hb-ship__who">
          <span className="hb-ship__title">Draft PR #128</span>
          <span className="hb-ship__sub">
            <Dot state="done" />
            Opened on GitHub
          </span>
        </span>
      </div>
      <div className="hb-ship__lines">
        <Bar w={118} h={5} tone={2} />
        <Bar w={84} h={5} tone={2} />
      </div>
    </div>
  );
}

// Where the travelling card sits: on the open slot of its lane, or past the
// board's right edge once it is done.
function spot(lane: Lane, rect: (name: string) => Rect): Rect {
  if (lane !== "done") return rect(`slot-${lane}`);
  const review = rect("slot-review");
  const step = review.x - rect("slot-progress").x;
  return { ...review, x: review.x + step * 1.3 };
}

function Scene({ s, rect, layout }: { s: S; rect: (name: string) => Rect; layout: "wide" | "compact" }) {
  if (layout === "compact") {
    const value = s.at === "open" ? 0 : s.at === "progress" ? 1 : 2;
    return (
      <>
        <div className="hb-frame">
          <Window className="hb-win" title="Board">
            <div className="hb-body">
              <Segmented className="hb-seg" value={value} items={LANES.map((l) => l.short)} />
              <div className="hb-list">
                <Collapse open={!s.hidden} reveal="drop">
                  <div className="hb-slot">
                    <TaskCard s={{ ...s, hidden: false }} compact />
                  </div>
                </Collapse>
                {FILL.review.map((f, i) => (
                  <Filler key={i} lines={[f.lines[0] + 90, f.lines[1] + 40]} meta={f.meta} />
                ))}
              </div>
            </div>
          </Window>
        </div>
        <span className="hb-rest" data-target="rest" style={{ left: 268, top: 312 }} />
      </>
    );
  }
  const at = spot(s.lane, rect);
  const board = rect("board");
  return (
    <>
      <div className="hb-frame">
        <Window
          className="hb-win"
          title="Board"
          sidebar={<Side />}
          sidebarWidth={112}
          toolbar={
            <span className="hb-tool">
              <Icon name="search" />
            </span>
          }
        >
          <div className="hb-body" data-target="board">
            <div className="hb-head">
              <span className="hb-channel">
                <span className="hb-hash">#</span>build
                <Icon name="chevron" size={12} />
              </span>
              <Bar w={132} h={6} tone={2} />
            </div>
            <div className="hb-lanes">
              {LANES.map((l) => (
                <div key={l.id} className="hb-lane">
                  <div className="hb-lane__head">
                    <span>{l.name}</span>
                    <span className="hb-count">
                      <Roll>{String(l.base + (s.slot === l.id ? 1 : 0))}</Roll>
                    </span>
                  </div>
                  {/* Room for the card, opened by a hidden copy of it, so the
                      lane makes exactly the room the card needs */}
                  <Collapse open={s.slot === l.id} data-target={`slot-${l.id}`}>
                    <div className="hb-slot">
                      <TaskCard s={s} ghost />
                    </div>
                  </Collapse>
                  {FILL[l.id as Exclude<Lane, "done">].map((f, i) => (
                    <Filler key={i} {...f} />
                  ))}
                </div>
              ))}
            </div>
            {at.width > 0 && <TaskCard s={s} className="hb-mover" style={{ width: at.width, transform: `translate(${at.x - board.x}px, ${at.y - board.y}px)` }} />}
          </div>
        </Window>
      </div>
      <Ship show={s.shipped} />
      <span className="hb-rest" data-target="rest" style={{ left: 640, top: 418 }} />
    </>
  );
}

/** The hero board, with its own pause control: it loops, so a reader can stop it (WCAG 2.2.2). */
export function HeroBoard() {
  const theme = usePageTheme();
  const reduced = usePrefersReducedMotion();
  const [paused, setPaused] = useState(false);
  return (
    <div className="hero-art">
      <div className="hero-art__stage" role="img" aria-label={LABEL}>
        <Stage<S>
          width={780}
          height={510}
          compact={{ below: 600, width: 358, height: 352 }}
          fill
          theme={theme}
          initial={initial}
          poster={POSTER}
          script={paused ? undefined : script}
          // Paused, the poster stands alone, as it does under reduced motion
          cursor={paused ? undefined : { at: "rest" }}
          className="ml hb"
        >
          {(s, rect, layout) => <Scene s={s} rect={rect} layout={layout} />}
        </Stage>
      </div>
      {reduced ? null : (
        <button type="button" className="hero-art__toggle" aria-pressed={paused} onClick={() => setPaused((p) => !p)}>
          {paused ? "Play the animation" : "Pause the animation"}
        </button>
      )}
    </div>
  );
}
