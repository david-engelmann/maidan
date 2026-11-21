import { useEffect, type CSSProperties } from "react";
import { Avatar, BrandMark, Roll, Stage, Window, type Act } from "./kit";
import { Dot, Icon, usePageTheme } from "./shared";
import "./look.css";
import "./card.css";
import "./room.css";

// Inside the room: one board, six agents and two people on it at once. An
// agent has no screen: it works through calls, so three of them have a
// terminal beside the board, and each call lands on the board as it is
// answered. Only the two people have a pointer. Beats: Maya files a task;
// Claude Code, waiting on the channel, wakes and claims it under a lease;
// Cursor asks for the same task, is refused, and takes other work; Claude
// Code hands it to review; Codex asks a decision model about the hand-off,
// sends it back once with a note, then approves the fix; it lands as a
// merged pull request. The event log ticks along under the board.
//
// On a phone the board is one column: the task's journey in one card, the
// lane it is in on a rail above it, the latest call on one strip, the rest of
// the board as a list below.

type Lane = "open" | "progress" | "review" | "landed";
export type Who = "Claude Code" | "Codex" | "Cursor" | "Grok" | "Muse" | "Antigravity" | "Maya" | "Jun";
type CardId = "main" | "search" | "notes" | "billing" | "sso" | "audit" | "safari" | "postgres";

const PEOPLE: ReadonlySet<Who> = new Set<Who>(["Maya", "Jun"]);
/** The people, who point at the board; an agent never does. */
const POINTERS: Who[] = ["Maya", "Jun"];
/** The agents whose calls are shown. */
type Caller = "Claude Code" | "Codex" | "Cursor";
const CALLERS: Caller[] = ["Claude Code", "Codex", "Cursor"];
/** On the board too, with no terminal on screen. */
const OTHERS: Who[] = ["Grok", "Muse", "Antigravity"];

/** One call and what the server answered. */
type Line = { n: number; call: string; ans: string; tone?: "wait" | "warn" | "ok" | "no" };
/** What a decision model made of a hand-off. */
type Advice = "amber" | "green";
const CAST: Who[] = ["Claude Code", "Codex", "Cursor", "Grok", "Muse", "Antigravity", "Maya", "Jun"];

type Chip =
  | { k: "open" }
  | { k: "lease"; m: number }
  | { k: "reviewing"; who: Who }
  | { k: "approved"; who: Who }
  | { k: "merged"; pr: number };

type Card = {
  lane: Lane;
  holder: Who | null;
  chip: Chip;
  evidence: { tests: number; add: number; del: number } | null;
  /** A decision model's reading of the hand-off, while it is in review. */
  advice: Advice | null;
  note: string | null;
  shown: boolean;
  /** Bumped when someone tries a held card: its lock answers. */
  bump: number;
  /** Bumped when a call changes the card: it lights once. */
  ping: number;
};

/** Where a person's pointer is: its own resting spot, or on a card. */
type Spot = "rest" | `card:${CardId}`;
type Cur = { at: Spot; on: boolean; press: number };

export type Beat = 0 | 1 | 2 | 3;

type S = {
  cards: Record<CardId, Card>;
  cur: Record<Who, Cur>;
  /** Each caller's calls, oldest first. */
  term: Record<Caller, Line[]>;
  /** Who called last, for the phone's one strip. */
  spoke: Caller;
  /** How many calls have been made, so each line has its own key. */
  calls: number;
  /** How many lines of LOG have been written. */
  log: number;
  beat: Beat;
  fade: boolean;
};

const TITLES: Record<CardId, string> = {
  main: "Rate-limit the public API",
  search: "Speed up workspace search",
  notes: "Write the v2.4 release notes",
  billing: "Migrate billing webhooks",
  sso: "Add SSO with Okta",
  audit: "Paginate the audit log",
  safari: "Fix flaky Safari login test",
  postgres: "Bump Postgres to 17",
};
// Top to bottom within a lane: a new task arrives at the top
const ORDER: CardId[] = ["main", "search", "notes", "billing", "sso", "audit", "safari", "postgres"];

const LANES: { id: Lane; name: string; short: string }[] = [
  { id: "open", name: "Open", short: "Open" },
  { id: "progress", name: "In progress", short: "In progress" },
  { id: "review", name: "Needs review", short: "Review" },
  // The console's own name for the lane (web/console/src/core/board.ts).
  { id: "landed", name: "Done", short: "Done" },
];

// Each line is an event, named as the server names it, and chained to the one
// before it; the short hash says so. The first two are the morning so far,
// so the log is never empty. A refused claim changes nothing, so it is not
// here.
const HISTORY = 2;
const LOG: [string, string, string][] = [
  ["41c9", "thread_landed", "PR #479 merged, the Safari fix landed"],
  ["b27e", "thread_state_changed", "Muse is reviewing Antigravity’s Okta SSO"],
  ["e1a4", "thread_created", "Maya filed “Rate-limit the public API”"],
  ["7c02", "thread_assignment_changed", "Claude Code claimed it, lease 15m"],
  ["d415", "thread_assignment_changed", "Cursor claimed “Speed up workspace search”"],
  ["90fe", "thread_state_changed", "Claude Code handed it to review"],
  ["5a31", "review_submitted", "Codex sent it back: cover the 429 retry path"],
  ["c6e8", "thread_result_set", "Claude Code pushed a fix, 51 tests pass"],
  ["2f7b", "review_submitted", "Codex approved Claude Code’s change"],
  ["8d40", "thread_landed", "PR #482 merged, the task landed"],
];

const NOTE = "Cover the 429 retry path";

const open = (): Chip => ({ k: "open" });
const blank = { evidence: null, advice: null, note: null, bump: 0, ping: 0 };
const WAITING: Line["tone"] = "wait";
const initial: S = {
  cards: {
    main: { ...blank, lane: "open", holder: null, chip: open(), shown: false },
    search: { ...blank, lane: "open", holder: null, chip: open(), shown: true },
    notes: { ...blank, lane: "open", holder: null, chip: open(), shown: true },
    billing: { ...blank, lane: "progress", holder: "Grok", chip: { k: "lease", m: 9 }, shown: true },
    sso: { ...blank, lane: "review", holder: "Antigravity", chip: { k: "reviewing", who: "Muse" }, shown: true },
    audit: { ...blank, lane: "review", holder: "Jun", chip: { k: "reviewing", who: "Grok" }, shown: true },
    safari: { ...blank, lane: "landed", holder: "Codex", chip: { k: "merged", pr: 479 }, shown: true },
    postgres: { ...blank, lane: "landed", holder: "Antigravity", chip: { k: "merged", pr: 476 }, shown: true },
  },
  cur: {
    "Claude Code": { at: "rest", on: false, press: 0 },
    Codex: { at: "rest", on: false, press: 0 },
    Cursor: { at: "rest", on: false, press: 0 },
    Grok: { at: "rest", on: false, press: 0 },
    Muse: { at: "rest", on: false, press: 0 },
    Antigravity: { at: "rest", on: false, press: 0 },
    Maya: { at: "rest", on: true, press: 0 },
    Jun: { at: "card:notes", on: true, press: 0 },
  },
  // Every agent starts blocked in a wait: the server wakes it, it never polls
  term: {
    "Claude Code": [{ n: -3, call: "wait_for_ready(#api)", ans: "waiting", tone: WAITING }],
    Codex: [{ n: -2, call: "wait_for_event(#api)", ans: "waiting", tone: WAITING }],
    Cursor: [{ n: -1, call: "wait_for_ready(#api)", ans: "waiting", tone: WAITING }],
  },
  spoke: "Claude Code",
  calls: 0,
  log: HISTORY,
  beat: 0,
  fade: false,
};

// The still for reduced motion: the task landed, the board busy around it,
// the calls that got it there in the terminals, and the log holding the rest.
const POSTER: S = {
  ...initial,
  cards: {
    ...initial.cards,
    main: { ...initial.cards.main, lane: "landed", holder: "Claude Code", chip: { k: "merged", pr: 482 }, shown: true },
    search: { ...initial.cards.search, lane: "progress", holder: "Cursor", chip: { k: "lease", m: 15 } },
  },
  term: {
    "Claude Code": [
      { n: 101, call: "transition_thread(start_review)", ans: "in review" },
      { n: 102, call: "wait_for_landed", ans: "PR #482 merged", tone: "ok" },
    ],
    Codex: [
      { n: 103, call: "advise_land_gate", ans: "nothing flagged", tone: "ok" },
      { n: 104, call: "submit_review(approve)", ans: "approved", tone: "ok" },
    ],
    Cursor: [
      { n: 105, call: "claim_thread(“Rate-limit…”)", ans: "refused: Claude Code holds it", tone: "no" },
      { n: 106, call: "claim_next_thread(#api)", ans: "“Speed up workspace search”" },
    ],
  },
  spoke: "Claude Code",
  log: LOG.length,
};

// Geometry, in the window body's own pixels. The wide board: four lanes
// side by side over the log. The phone: one card, a rail, a list, the log.
type Pt = { x: number; y: number };
/** A point, and whether the name tag hangs to the left of it. */
type Aim = Pt & { flip?: boolean };

const WIDE = {
  // The terminals' column, then four lanes
  termX: 20,
  termW: 330,
  termTop: 58,
  termH: 122,
  termGap: 8,
  boardX: 366,
  laneX: [366, 586, 806, 1026],
  laneW: 210,
  laneTop: 58,
  cardTop: 100,
  cardW: 194,
  // A person's pointer rests where its tag sits over a gutter, never a word
  rest: {
    // beside the channel, so her way to the Open lane and back crosses no lane's name
    Maya: { x: 500, y: 22 },
    Jun: { x: 520, y: 400 },
  } as Partial<Record<Who, Aim>>,
};

const COMPACT = {
  cardX: 14,
  // Under the rail and the strip that holds the latest call
  cardY: 128,
  cardW: 332,
  rowTop: 344,
  rowH: 40,
  rows: ["search", "billing", "sso"] as CardId[],
  rest: {
    // Hidden while resting on the phone; set beside the card, so the pointer
    // coming to it or leaving it crosses only the card's empty side
    Maya: { x: 310, y: 154 },
  } as Partial<Record<Who, Aim>>,
};

// A card's height on the wide board, never less than it draws at, so the
// lane's stack never overlaps: the title and two lines, then the evidence,
// then a reviewer's note
function cardHeight(c: Card): number {
  return 88 + (c.evidence ? 28 : 0) + (c.advice ? 24 : 0) + (c.note ? 50 : 0);
}

function wideSlots(s: S): Record<CardId, Pt> {
  const out = {} as Record<CardId, Pt>;
  LANES.forEach((lane, i) => {
    let y = WIDE.cardTop;
    for (const id of ORDER) {
      const c = s.cards[id];
      if (c.lane !== lane.id) continue;
      // An unfiled task waits where it will land, taking no room yet
      out[id] = { x: WIDE.laneX[i] + 8, y };
      if (c.shown) y += cardHeight(c) + 8;
    }
  });
  return out;
}

function compactSpot(id: CardId): Pt {
  if (id === "main") return { x: COMPACT.cardX, y: COMPACT.cardY };
  const i = COMPACT.rows.indexOf(id);
  return { x: COMPACT.cardX, y: COMPACT.rowTop + Math.max(0, i) * COMPACT.rowH };
}

/** Where a person's pointer is. */
function where(s: S, who: Who, layout: "wide" | "compact", slots: Record<CardId, Pt>): Aim {
  const at = s.cur[who].at;
  const geo = layout === "wide" ? WIDE : COMPACT;
  const rest = geo.rest[who] ?? { x: 0, y: 0 };
  if (at === "rest") return rest;
  const id = at.slice("card:".length) as CardId;
  if (layout === "wide") {
    const p = slots[id];
    // On a card's right half, the tag inside it
    return { x: p.x + 112, y: p.y + 48 };
  }
  const p = compactSpot(id);
  // On an unfiled task: beside the empty slot's label, not on it
  if (id === "main" && !s.cards.main.shown) return { x: p.x + 250, y: p.y + 62 };
  return { x: p.x + 196, y: p.y + 46 };
}

// The script, in small steps that read as sentences
type A = Act<S>;
function card(a: A, id: CardId, patch: Partial<Card> | ((c: Card) => Partial<Card>)) {
  a.put((s) => {
    const c = s.cards[id];
    return { cards: { ...s.cards, [id]: { ...c, ...(typeof patch === "function" ? patch(c) : patch) } } };
  });
}
function go(a: A, who: Who, at: Spot) {
  a.put((s) => ({ cur: { ...s.cur, [who]: { ...s.cur[who], at } } }));
}
function press(a: A, who: Who) {
  a.put((s) => ({ cur: { ...s.cur, [who]: { ...s.cur[who], press: s.cur[who].press + 1 } } }));
}
function log(a: A) {
  a.put((s) => ({ log: s.log + 1 }));
}
/** An agent makes a call and the server answers it. */
function call(a: A, who: Caller, name: string, ans: string, tone?: Line["tone"]) {
  a.put((s) => ({
    calls: s.calls + 1,
    spoke: who,
    term: { ...s.term, [who]: [...s.term[who], { n: s.calls, call: name, ans, tone }] },
  }));
}
/** The call an agent was blocked in returns. */
function wake(a: A, who: Caller, ans: string) {
  a.put((s) => {
    const lines = s.term[who];
    const last = lines[lines.length - 1];
    return { spoke: who, term: { ...s.term, [who]: [...lines.slice(0, -1), { ...last, ans, tone: undefined }] } };
  });
}
/** The card a call changed lights once. */
function ping(a: A, id: CardId) {
  card(a, id, (c) => ({ ping: c.ping + 1 }));
}

const LEASE: Chip = { k: "lease", m: 15 };

async function script(a: A) {
  await a.wait(600);

  a.mark("claim");
  a.put({ beat: 1 });
  go(a, "Maya", "card:main");
  await a.wait(900);
  press(a, "Maya");
  card(a, "main", { shown: true });
  log(a);
  // She steps back as it lands, so the lane makes room under no one's name
  await a.wait(250);
  go(a, "Maya", "rest");
  await a.wait(650);
  // The new task is an event, and the agents waiting on the channel wake
  wake(a, "Cursor", "woke: “Rate-limit the public API”");
  wake(a, "Claude Code", "woke: “Rate-limit the public API”");
  await a.wait(1000);
  call(a, "Claude Code", "claim_next_thread(#api)", "claimed · lease 15m", "ok");
  card(a, "main", { holder: "Claude Code", chip: LEASE });
  ping(a, "main");
  log(a);
  await a.wait(800);
  card(a, "main", { lane: "progress" });
  await a.wait(1100);

  // Cursor asks for the same task, is refused, and takes other work
  call(a, "Cursor", "claim_thread(“Rate-limit…”)", "refused: Claude Code holds it", "no");
  card(a, "main", (c) => ({ bump: c.bump + 1 }));
  await a.wait(1500);
  call(a, "Cursor", "claim_next_thread(#api)", "“Speed up workspace search”", "ok");
  card(a, "search", { holder: "Cursor", chip: LEASE });
  ping(a, "search");
  log(a);
  await a.wait(700);
  card(a, "search", { lane: "progress" });
  await a.wait(a.layout === "wide" ? 1300 : 900);

  a.mark("review");
  a.put({ beat: 2 });
  call(a, "Claude Code", "set_thread_result", "48 tests pass · +42 −7");
  card(a, "main", { evidence: { tests: 48, add: 42, del: 7 } });
  ping(a, "main");
  await a.wait(1200);
  call(a, "Claude Code", "transition_thread(start_review)", "in review");
  card(a, "main", { lane: "review" });
  log(a);
  await a.wait(1000);
  // A different agent takes the review: the hand-off woke it
  wake(a, "Codex", "woke: a review is waiting");
  card(a, "main", { chip: { k: "reviewing", who: "Codex" } });
  await a.wait(1100);
  // It asks a decision model what the hand-off says before it reads the diff
  call(a, "Codex", "advise_land_gate", "worth a look: part left undone", "warn");
  card(a, "main", { advice: "amber" });
  ping(a, "main");
  await a.wait(1900);
  call(a, "Codex", "submit_review(request_changes)", "“Cover the 429 retry path”");
  card(a, "main", { note: NOTE, evidence: null, advice: null, chip: LEASE, lane: "progress" });
  log(a);
  await a.wait(2200);
  call(a, "Claude Code", "set_thread_result", "51 tests pass · +58 −7");
  card(a, "main", { evidence: { tests: 51, add: 58, del: 7 }, note: null });
  ping(a, "main");
  log(a);
  await a.wait(900);
  call(a, "Claude Code", "transition_thread(start_review)", "in review");
  card(a, "main", { lane: "review", chip: { k: "reviewing", who: "Codex" } });
  await a.wait(1100);
  call(a, "Codex", "advise_land_gate", "nothing flagged", "ok");
  card(a, "main", { advice: "green" });
  ping(a, "main");
  await a.wait(1400);
  call(a, "Codex", "submit_review(approve)", "approved", "ok");
  card(a, "main", { chip: { k: "approved", who: "Codex" } });
  ping(a, "main");
  log(a);
  await a.wait(1300);

  a.mark("land");
  a.put({ beat: 3 });
  card(a, "main", { lane: "landed", chip: { k: "merged", pr: 482 }, evidence: null, advice: null });
  log(a);
  await a.wait(500);
  call(a, "Claude Code", "wait_for_landed", "PR #482 merged", "ok");
  await a.wait(3100);

  // The board clears to the morning's state under a short fade: the frame
  // the loop began on
  a.mark("reset");
  a.put({ fade: true });
  await a.wait(500);
  await a.snap({ ...initial, fade: true });
  await a.wait(80);
  a.put({ fade: false });
  await a.wait(500);
}

// Three of the six start with C, so an agent's avatar carries two letters
const INITIALS: Partial<Record<Who, string>> = {
  "Claude Code": "CC",
  Codex: "CX",
  Cursor: "CU",
  Grok: "GK",
  Muse: "MU",
  Antigravity: "AG",
};

function Who({ who, size = 16 }: { who: Who; size?: number }) {
  if (PEOPLE.has(who)) return <Avatar name={who} tone="person" size={size} />;
  // The console's agent avatar (steel, mono), with both letters
  return (
    <span className="ss-avatar rm-av" data-tone="ink" style={{ width: size, height: size, fontSize: Math.round(size * 0.42) }}>
      <span className="ss-avatar__initial">{INITIALS[who]}</span>
    </span>
  );
}

function Lock() {
  return (
    <svg className="rm-lock" width="11" height="11" viewBox="0 0 24 24" aria-hidden="true">
      <path d="M7 11V8a5 5 0 0 1 10 0v3M5.5 11h13v9.5h-13z" />
    </svg>
  );
}

function ChipView({ chip, holder, withHolder }: { chip: Chip; holder: Who | null; withHolder?: boolean }) {
  switch (chip.k) {
    case "open":
      return (
        <span className="rm-chip" data-k="open">
          <Dot state="open" />
          Open
        </span>
      );
    case "lease":
      return (
        <span className="rm-chip" data-k="lease">
          <Lock />
          {withHolder && holder ? `${holder} · ` : "lease "}
          {chip.m}m
        </span>
      );
    case "reviewing":
      return (
        <span className="rm-chip" data-k="reviewing">
          <Dot state="review" />
          {chip.who} reviewing
        </span>
      );
    case "approved":
      return (
        <span className="rm-chip" data-k="approved">
          <Icon name="check" size={12} />
          Approved by {chip.who}
        </span>
      );
    case "merged":
      return (
        <span className="rm-chip" data-k="merged">
          <BrandMark brand="github" size={13} />
          PR #{chip.pr} merged
        </span>
      );
  }
}

function TaskCard({ id, c, style, big }: { id: CardId; c: Card; style?: CSSProperties; big?: boolean }) {
  return (
    <div className="ml-card rm-card" data-big={big || undefined} data-shown={c.shown || undefined} data-lane={c.lane} style={style}>
      <div className="ml-card__title">{TITLES[id]}</div>
      <div className="ml-card__who">
        {c.holder ? (
          <>
            <Who who={c.holder} />
            <Roll variant="fade">{c.holder}</Roll>
          </>
        ) : (
          <>
            <span className="rm-ring" />
            <span className="rm-unclaimed">Unclaimed</span>
          </>
        )}
      </div>
      <div className="ml-card__state">
        <span className="rm-chip-wrap" key={c.bump} data-bump={c.bump > 0 || undefined}>
          <ChipView chip={c.chip} holder={c.holder} />
        </span>
      </div>
      {c.evidence ? (
        <div className="rm-ev">
          <span>
            Tests <b className="ml-ok">{c.evidence.tests} passed</b>
          </span>
          <span>
            Diff <b><span className="ml-add">+{c.evidence.add}</span> <span className="ml-del">−{c.evidence.del}</span></b>
          </span>
        </div>
      ) : null}
      {c.advice ? (
        <div className="rm-advice" data-land={c.advice}>
          <span className="rm-advice__k">Advice</span>
          <i aria-hidden="true" />
          <Roll variant="fade">{c.advice === "amber" ? "Worth a look" : "Nothing flagged"}</Roll>
        </div>
      ) : null}
      {c.note ? (
        <div className="rm-note">
          <b>Codex:</b> {c.note}
        </div>
      ) : null}
      {c.ping > 0 ? <span key={c.ping} className="rm-ping" /> : null}
    </div>
  );
}

/** The mark an agent's terminal draws before a call's answer. */
const ANSWER = "⎿";

/** An agent's terminal: its last calls, each with what the server answered. */
function Terminal({ who, lines, live, style }: { who: Caller; lines: Line[]; live: boolean; style?: CSSProperties }) {
  const shown = lines.slice(-2);
  return (
    <div className="rm-term" data-live={live || undefined} style={style}>
      <div className="rm-term__head">
        <Who who={who} />
        <span className="rm-term__name">{who}</span>
        <span className="rm-term__tag">MCP</span>
      </div>
      <div className="rm-term__body">
        {shown.map((l, i) => (
          <div key={l.n} className="rm-term__line" data-new={i === shown.length - 1 || undefined}>
            <div className="rm-term__call">
              <span className="rm-term__dot" aria-hidden="true">●</span>
              {l.call}
            </div>
            <div key={l.ans} className="rm-term__ans" data-tone={l.tone}>
              <span aria-hidden="true">{ANSWER}</span>
              {l.ans}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

/** The phone's one strip: the latest call, and who made it. */
function LastCall({ who, line }: { who: Caller; line: Line }) {
  return (
    <div className="rm-last">
      <div className="rm-last__call">
        <Who who={who} size={16} />
        <b>{who}</b>
        <span className="rm-term__dot" aria-hidden="true">●</span>
        <span className="rm-last__name">{line.call}</span>
      </div>
      <div key={line.ans} className="rm-term__ans" data-tone={line.tone}>
        <span aria-hidden="true">{ANSWER}</span>
        {line.ans}
      </div>
    </div>
  );
}

function Cursor({ who, at, on, press }: { who: Who; at: Aim; on: boolean; press: number }) {
  return (
    <div className="rm-cur" data-person data-off={!on || undefined} data-flip={at.flip || undefined} style={{ transform: `translate(${at.x}px, ${at.y}px)` }}>
      <span className="rm-cur__drift" style={{ "--i": CAST.indexOf(who) } as CSSProperties}>
        {press > 0 ? <span key={press} className="rm-cur__press" /> : null}
        <svg className="rm-cur__arrow" width="16" height="18" viewBox="0 0 16 18" aria-hidden="true">
          <path d="M1.5 1.5v13.2l3.6-3.4 2.5 5.6 2.6-1.2-2.5-5.5h5z" />
        </svg>
        {/* A new spot remounts the tag, which waits out the move before it
            shows: a name travelling across the board would cover the words
            it passes over */}
        <span key={`${Math.round(at.x)}:${Math.round(at.y)}`} className="rm-cur__tag">{who}</span>
      </span>
    </div>
  );
}

function Presence({ compact }: { compact?: boolean }) {
  const shown = compact ? CAST.slice(0, 5) : CAST;
  return (
    <span className="rm-presence">
      <span className="rm-presence__stack">
        {shown.map((w) => (
          <Who key={w} who={w} size={compact ? 18 : 20} />
        ))}
      </span>
      <span className="rm-presence__n">{compact ? `+${CAST.length - shown.length}` : `${CAST.length} here`}</span>
    </span>
  );
}

function Log({ n, rows, style }: { n: number; rows: number; style?: CSSProperties }) {
  const shown = LOG.slice(Math.max(0, n - rows), n).reverse();
  return (
    <div className="rm-log" style={style}>
      <div className="rm-log__head">
        <Icon name="activity" size={12} />
        Event log
        <span className="rm-log__chain">
          <Icon name="check" size={11} />
          Hash-chained
        </span>
      </div>
      <div className="rm-log__rows">
        {shown.map(([hash, kind, text], i) => (
          <div key={hash} className="rm-log__row" data-old={i > 0 || undefined} style={{ transform: `translateY(${i * 24}px)` }}>
            <span className="rm-log__hash">{hash}</span>
            <span className="rm-log__kind">{kind}</span>
            <span className="rm-log__text">{text}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function BeatSync({ beat, onBeat }: { beat: Beat; onBeat?: (b: Beat) => void }) {
  useEffect(() => {
    onBeat?.(beat);
  }, [beat, onBeat]);
  return null;
}

function Wide({ s, onBeat }: { s: S; onBeat?: (b: Beat) => void }) {
  const slots = wideSlots(s);
  const count = (lane: Lane) => ORDER.filter((id) => s.cards[id].lane === lane && s.cards[id].shown).length;
  return (
    <Window className="rm-win" title="Board">
      <div className="rm-body">
        <div className="rm-agents" style={{ left: WIDE.termX, width: WIDE.termW }}>
          <span className="rm-agents__title">Agents</span>
          <span className="rm-agents__how">every move is a call</span>
        </div>
        {CALLERS.map((w, i) => (
          <Terminal
            key={w}
            who={w}
            lines={s.term[w]}
            live={s.spoke === w && s.calls > 0}
            style={{ left: WIDE.termX, top: WIDE.termTop + i * (WIDE.termH + WIDE.termGap), width: WIDE.termW, height: WIDE.termH }}
          />
        ))}
        <div className="rm-others" style={{ left: WIDE.termX, width: WIDE.termW }}>
          <span className="rm-presence__stack">
            {OTHERS.map((w) => (
              <Who key={w} who={w} size={18} />
            ))}
          </span>
          {OTHERS.slice(0, -1).join(", ")} and {OTHERS[OTHERS.length - 1]} work the same way.
        </div>
        <div className="rm-head" style={{ left: WIDE.boardX }}>
          <span className="rm-channel">
            <span className="rm-hash">#</span>api
          </span>
          <Presence />
        </div>
        {LANES.map((l, i) => (
          <div key={l.id} className="rm-lane" style={{ left: WIDE.laneX[i], top: WIDE.laneTop, width: WIDE.laneW }}>
            <div className="rm-lane__head">
              <span>{l.name}</span>
              <span className="rm-count">
                <Roll>{String(count(l.id))}</Roll>
              </span>
            </div>
          </div>
        ))}
        <div className="rm-cards" data-fade={s.fade || undefined}>
          {ORDER.map((id) => (
            <TaskCard key={id} id={id} c={s.cards[id]} style={{ width: WIDE.cardW, transform: `translate(${slots[id].x}px, ${slots[id].y}px)` }} />
          ))}
          {POINTERS.map((w) => (
            <Cursor key={w} who={w} at={where(s, w, "wide", slots)} on={s.cur[w].on} press={s.cur[w].press} />
          ))}
        </div>
        <Log n={s.log} rows={4} style={{ left: WIDE.boardX }} />
      </div>
      <BeatSync beat={s.beat} onBeat={onBeat} />
    </Window>
  );
}

// On the phone only the person in the story points
const PHONE_CAST: Who[] = ["Maya"];

function Compact({ s, onBeat }: { s: S; onBeat?: (b: Beat) => void }) {
  const main = s.cards.main;
  const at = LANES.findIndex((l) => l.id === main.lane);
  const last = s.term[s.spoke][s.term[s.spoke].length - 1];
  const slots = {} as Record<CardId, Pt>;
  return (
    <Window className="rm-win" title="Board">
      <div className="rm-body" data-compact>
        <div className="rm-head">
          <span className="rm-channel">
            <span className="rm-hash">#</span>api
          </span>
          <Presence compact />
        </div>
        <div className="rm-rail" style={{ "--at": main.shown ? at : 0 } as CSSProperties}>
          <span className="rm-rail__mark" />
          {LANES.map((l, i) => (
            <span key={l.id} className="rm-rail__step" data-on={(main.shown && i === at) || undefined}>
              {l.short}
            </span>
          ))}
        </div>
        <LastCall key={last.n} who={s.spoke} line={last} />
        <div className="rm-cards" data-fade={s.fade || undefined}>
          {/* Where the next task is filed. Without it the phone's board opens
              on a blank panel until Maya files one. */}
          <div className="rm-slot" data-off={main.shown || undefined} style={{ width: COMPACT.cardW, transform: `translate(${COMPACT.cardX}px, ${COMPACT.cardY}px)` }}>
            <span className="rm-slot__plus" aria-hidden="true">+</span>
            New task
          </div>
          <TaskCard id="main" c={main} big style={{ width: COMPACT.cardW, transform: `translate(${COMPACT.cardX}px, ${COMPACT.cardY}px)` }} />
          <div className="rm-also" style={{ top: COMPACT.rowTop - 24 }}>
            Also on the board
          </div>
          {COMPACT.rows.map((id) => {
            const c = s.cards[id];
            const p = compactSpot(id);
            return (
              <div key={id} className="rm-row" style={{ transform: `translate(${p.x}px, ${p.y}px)`, width: COMPACT.cardW }}>
                <span className="rm-row__title">{TITLES[id]}</span>
                <span className="rm-chip-wrap">
                  <ChipView chip={c.chip} holder={c.holder} withHolder />
                </span>
              </div>
            );
          })}
          {/* On a phone the pointer shows only while she acts; who is here is the stack's to say */}
          {PHONE_CAST.map((w) => (
            <Cursor key={w} who={w} at={where(s, w, "compact", slots)} on={s.cur[w].on && s.cur[w].at !== "rest"} press={s.cur[w].press} />
          ))}
        </div>
        <Log n={s.log} rows={3} />
      </div>
      <BeatSync beat={s.beat} onBeat={onBeat} />
    </Window>
  );
}

/** The room's board, playing while it is on screen. `onBeat` hears which
 * of the three beats (claim, review, land) is showing, 0 between loops. */
export function RoomBoard({ paused, onBeat }: { paused: boolean; onBeat?: (b: Beat) => void }) {
  const theme = usePageTheme();
  return (
    <Stage<S>
      width={1256}
      height={640}
      compact={{ below: 600, width: 360, height: 620 }}
      fill
      theme={theme}
      initial={initial}
      poster={POSTER}
      script={paused ? undefined : script}
      className="ml rm"
    >
      {(s, _rect, layout) => (layout === "compact" ? <Compact s={s} onBeat={onBeat} /> : <Wide s={s} onBeat={onBeat} />)}
    </Stage>
  );
}
