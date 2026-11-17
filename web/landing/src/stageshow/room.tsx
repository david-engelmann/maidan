import { useEffect, type CSSProperties } from "react";
import { Avatar, BrandMark, Roll, Stage, Window, type Act } from "./kit";
import { Dot, Icon, usePageTheme } from "./shared";
import "./look.css";
import "./card.css";
import "./room.css";

// Inside the room: one board, six agents and two people on it at once, the
// way a multiplayer tool shows presence. Beats: Maya files a task; Claude
// Code claims it and it carries a lease lock; Cursor comes for it, sees the
// lock and takes other work; Claude Code hands it to review; Codex sends it
// back once with a note, then approves the fix; it lands as a merged pull
// request. The event log ticks along under the board.
//
// On a phone the board is one column: the task's journey in one card, the
// lane it is in on a rail above it, the rest of the board as a list below.

type Lane = "open" | "progress" | "review" | "landed";
export type Who = "Claude Code" | "Codex" | "Cursor" | "Grok" | "Muse" | "Antigravity" | "Maya" | "Jun";
type CardId = "main" | "search" | "notes" | "billing" | "sso" | "audit" | "safari" | "postgres";

const PEOPLE: ReadonlySet<Who> = new Set<Who>(["Maya", "Jun"]);
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
  note: string | null;
  shown: boolean;
  /** Bumped when someone tries a held card: its lock answers. */
  bump: number;
};

/** Where a cursor is: its own resting spot, on a card, or passing one by. */
type Spot = "rest" | `card:${CardId}` | `near:${CardId}`;
type Cur = { at: Spot; on: boolean; press: number };

export type Beat = 0 | 1 | 2 | 3;

type S = {
  cards: Record<CardId, Card>;
  cur: Record<Who, Cur>;
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
  { id: "landed", name: "Landed", short: "Landed" },
];

// Each line is chained to the one before it; the short hash says so.
// The first two are the morning so far, so the log is never empty.
const HISTORY = 2;
const LOG: [string, string][] = [
  ["41c9", "PR #479 merged, the Safari fix landed"],
  ["b27e", "Muse is reviewing Antigravity’s Okta SSO"],
  ["e1a4", "Maya filed “Rate-limit the public API”"],
  ["7c02", "Claude Code claimed it, lease 15m"],
  ["3b9d", "Cursor passed it by: Claude Code holds it"],
  ["d415", "Cursor claimed “Speed up workspace search”"],
  ["90fe", "Claude Code handed it to review"],
  ["5a31", "Codex sent it back: cover the 429 retry path"],
  ["c6e8", "Claude Code pushed a fix, 51 tests pass"],
  ["2f7b", "Codex approved Claude Code’s change"],
  ["8d40", "PR #482 merged, the task landed"],
];

const NOTE = "Cover the 429 retry path";

const open = (): Chip => ({ k: "open" });
const initial: S = {
  cards: {
    main: { lane: "open", holder: null, chip: open(), evidence: null, note: null, shown: false, bump: 0 },
    search: { lane: "open", holder: null, chip: open(), evidence: null, note: null, shown: true, bump: 0 },
    notes: { lane: "open", holder: null, chip: open(), evidence: null, note: null, shown: true, bump: 0 },
    billing: { lane: "progress", holder: "Grok", chip: { k: "lease", m: 9 }, evidence: null, note: null, shown: true, bump: 0 },
    sso: { lane: "review", holder: "Antigravity", chip: { k: "reviewing", who: "Muse" }, evidence: null, note: null, shown: true, bump: 0 },
    audit: { lane: "review", holder: "Jun", chip: { k: "reviewing", who: "Grok" }, evidence: null, note: null, shown: true, bump: 0 },
    safari: { lane: "landed", holder: "Codex", chip: { k: "merged", pr: 479 }, evidence: null, note: null, shown: true, bump: 0 },
    postgres: { lane: "landed", holder: "Antigravity", chip: { k: "merged", pr: 476 }, evidence: null, note: null, shown: true, bump: 0 },
  },
  cur: {
    "Claude Code": { at: "rest", on: true, press: 0 },
    Codex: { at: "rest", on: true, press: 0 },
    Cursor: { at: "rest", on: true, press: 0 },
    Grok: { at: "card:billing", on: true, press: 0 },
    Muse: { at: "card:sso", on: true, press: 0 },
    Antigravity: { at: "rest", on: true, press: 0 },
    Maya: { at: "rest", on: true, press: 0 },
    Jun: { at: "card:notes", on: true, press: 0 },
  },
  log: HISTORY,
  beat: 0,
  fade: false,
};

// The still for reduced motion: the task landed, the board busy around it,
// and the log holding how it got there.
const POSTER: S = {
  ...initial,
  cards: {
    ...initial.cards,
    main: { ...initial.cards.main, lane: "landed", holder: "Claude Code", chip: { k: "merged", pr: 482 }, shown: true },
    search: { ...initial.cards.search, lane: "progress", holder: "Cursor", chip: { k: "lease", m: 15 } },
  },
  cur: { ...initial.cur, "Claude Code": { at: "card:main", on: true, press: 0 }, Codex: { at: "rest", on: true, press: 0 }, Cursor: { at: "card:search", on: true, press: 0 }, Jun: { at: "card:notes", on: true, press: 0 } },
  log: LOG.length,
};

// Geometry, in the window body's own pixels. The wide board: four lanes
// side by side over the log. The phone: one card, a rail, a list, the log.
type Pt = { x: number; y: number };
/** A point, and whether the name tag hangs to the left of it. */
type Aim = Pt & { flip?: boolean };

const WIDE = {
  laneX: [20, 286, 552, 818],
  laneW: 254,
  laneTop: 58,
  cardTop: 100,
  cardW: 238,
  // Resting in the foot of a lane, below the most cards it ever holds, so a
  // tag there sits over the gutter and never over a word
  rest: {
    "Claude Code": { x: 420, y: 418 },
    Codex: { x: 716, y: 418 },
    Cursor: { x: 178, y: 418 },
    Grok: { x: 480, y: 330 },
    Muse: { x: 760, y: 330 },
    Antigravity: { x: 956, y: 418 },
    Maya: { x: 432, y: 22 },
    Jun: { x: 200, y: 400 },
  } as Record<Who, Aim>,
};

const COMPACT = {
  cardX: 14,
  cardY: 92,
  cardW: 332,
  rowTop: 300,
  rowH: 42,
  rows: ["search", "billing", "sso", "safari"] as CardId[],
  rest: {
    "Claude Code": { x: 166, y: 248 },
    Codex: { x: 232, y: 470 },
    Cursor: { x: 334, y: 252, flip: true },
    Grok: { x: 0, y: 0 },
    Muse: { x: 0, y: 0 },
    Antigravity: { x: 0, y: 0 },
    Maya: { x: 116, y: 10 },
    Jun: { x: 0, y: 0 },
  } as Record<Who, Aim>,
};

// A card's height on the wide board, never less than it draws at, so the
// lane's stack never overlaps: the title and two lines, then the evidence,
// then a reviewer's note
function cardHeight(c: Card): number {
  return 88 + (c.evidence ? 28 : 0) + (c.note ? 36 : 0);
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

/** Where a cursor points. */
function where(s: S, who: Who, layout: "wide" | "compact", slots: Record<CardId, Pt>): Aim {
  const at = s.cur[who].at;
  const geo = layout === "wide" ? WIDE : COMPACT;
  if (at === "rest") return geo.rest[who];
  const [kind, id] = at.split(":") as ["card" | "near", CardId];
  if (layout === "wide") {
    const p = slots[id];
    // On a card: its right half, the tag inside it; passing: beside its lock
    return kind === "card" ? { x: p.x + 136, y: p.y + 48 } : { x: p.x + 112, y: p.y + 58 };
  }
  const p = compactSpot(id);
  if (id === "main") return kind === "card" ? { x: p.x + 196, y: p.y + 46 } : { x: p.x + 118, y: p.y + 84 };
  // A list row: on its chip, the tag hanging back over the row's own title
  return { x: p.x + 236, y: p.y + 22, flip: true };
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
  await a.wait(750);
  go(a, "Claude Code", "card:main");
  await a.wait(1000);
  press(a, "Claude Code");
  card(a, "main", { holder: "Claude Code", chip: { k: "lease", m: 15 } });
  log(a);
  await a.wait(800);
  card(a, "main", { lane: "progress" });
  await a.wait(1000);
  // Claude Code gets to work, and the lock is what holds the card
  go(a, "Claude Code", "rest");
  await a.wait(500);

  // Cursor comes for it, finds the lock, and takes other work
  go(a, "Cursor", "near:main");
  await a.wait(1000);
  card(a, "main", (c) => ({ bump: c.bump + 1 }));
  log(a);
  await a.wait(900);
  if (a.layout === "wide") {
    go(a, "Cursor", "card:search");
    await a.wait(1000);
    press(a, "Cursor");
  } else {
    // On the phone the claim shows on the task's row; a cursor there would
    // hang its name over the next row's words
    go(a, "Cursor", "rest");
    await a.wait(700);
  }
  card(a, "search", { holder: "Cursor", chip: { k: "lease", m: 15 } });
  log(a);
  await a.wait(700);
  card(a, "search", { lane: "progress" });
  await a.wait(a.layout === "wide" ? 1300 : 900);

  a.mark("review");
  a.put({ beat: 2 });
  go(a, "Claude Code", "card:main");
  await a.wait(1000);
  card(a, "main", { evidence: { tests: 48, add: 42, del: 7 } });
  await a.wait(700);
  card(a, "main", { lane: "review" });
  log(a);
  await a.wait(900);
  // The author steps back, and a different agent takes the review
  go(a, "Claude Code", "rest");
  go(a, "Codex", "card:main");
  await a.wait(1000);
  card(a, "main", { chip: { k: "reviewing", who: "Codex" } });
  await a.wait(1500);
  press(a, "Codex");
  card(a, "main", { note: NOTE, evidence: null, chip: { k: "lease", m: 15 }, lane: "progress" });
  go(a, "Codex", "rest");
  go(a, "Claude Code", "card:main");
  log(a);
  await a.wait(2200);
  card(a, "main", { evidence: { tests: 51, add: 58, del: 7 }, note: null });
  log(a);
  await a.wait(800);
  card(a, "main", { lane: "review", chip: { k: "reviewing", who: "Codex" } });
  await a.wait(500);
  go(a, "Claude Code", "rest");
  go(a, "Codex", "card:main");
  await a.wait(1300);
  press(a, "Codex");
  card(a, "main", { chip: { k: "approved", who: "Codex" } });
  log(a);
  await a.wait(1300);

  a.mark("land");
  a.put({ beat: 3 });
  card(a, "main", { lane: "landed", chip: { k: "merged", pr: 482 }, evidence: null });
  log(a);
  await a.wait(1000);
  go(a, "Codex", "rest");
  await a.wait(2600);

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
      {c.note ? (
        <div className="rm-note">
          <b>Codex:</b> {c.note}
        </div>
      ) : null}
    </div>
  );
}

function Cursor({ who, at, on, press }: { who: Who; at: Aim; on: boolean; press: number }) {
  const person = PEOPLE.has(who);
  return (
    <div className="rm-cur" data-person={person || undefined} data-off={!on || undefined} data-flip={at.flip || undefined} style={{ transform: `translate(${at.x}px, ${at.y}px)` }}>
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

function Log({ n, rows }: { n: number; rows: number }) {
  const shown = LOG.slice(Math.max(0, n - rows), n).reverse();
  return (
    <div className="rm-log">
      <div className="rm-log__head">
        <Icon name="activity" size={12} />
        Event log
        <span className="rm-log__chain">
          <Icon name="check" size={11} />
          Hash-chained
        </span>
      </div>
      <div className="rm-log__rows">
        {shown.map(([hash, text], i) => (
          <div key={hash} className="rm-log__row" data-old={i > 0 || undefined} style={{ transform: `translateY(${i * 24}px)` }}>
            <span className="rm-log__hash">{hash}</span>
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
        <div className="rm-head">
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
          {CAST.map((w) => (
            <Cursor key={w} who={w} at={where(s, w, "wide", slots)} on={s.cur[w].on} press={s.cur[w].press} />
          ))}
        </div>
        <Log n={s.log} rows={4} />
      </div>
      <BeatSync beat={s.beat} onBeat={onBeat} />
    </Window>
  );
}

// On the phone only the cursors in the story move; the rest are on the list
const PHONE_CAST: Who[] = ["Claude Code", "Codex", "Cursor", "Maya"];

function Compact({ s, onBeat }: { s: S; onBeat?: (b: Beat) => void }) {
  const main = s.cards.main;
  const at = LANES.findIndex((l) => l.id === main.lane);
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
        <div className="rm-cards" data-fade={s.fade || undefined}>
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
          {/* On a phone a cursor shows only while it acts; who is here is the stack's to say */}
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
      width={1100}
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
