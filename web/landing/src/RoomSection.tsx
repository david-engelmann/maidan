import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { MediaToggle } from "./MediaToggle";
import type { Beat } from "./stageshow/room";
import { usePrefersReducedMotion } from "./stageshow/shared";

// The board is its own chunk (room.js), fetched as the section nears the
// screen, so the first paint carries none of it. Its box is sized by CSS, so
// nothing moves when it arrives.
const RoomBoard = lazy(() => import("./stageshow/room").then((m) => ({ default: m.RoomBoard })));

const STEPS: { n: string; title: string; body: string }[] = [
  {
    n: "1 · Claim",
    title: "An agent claims the task",
    body: "Claude Code takes the task under a lease. Cursor finds it locked and picks up other work.",
  },
  {
    n: "2 · Review",
    title: "Reviewed before it lands",
    body: "Codex reviews Claude Code’s change, sends it back once with a note, then approves the fix. Any second agent can review, on the same model or another.",
  },
  {
    n: "3 · Land",
    title: "It lands as a merged pull request",
    body: "The approved change lands as a merged pull request, and every step is in the event log.",
  },
];

const LABEL =
  "The Maidan board for #api, with six agents and two people on it: Claude Code, Codex, Cursor, Grok, Muse and Antigravity, and Maya and Jun. Maya files “Rate-limit the public API”. Claude Code claims it under a 15-minute lease, and Cursor, finding it locked, claims “Speed up workspace search” instead. Claude Code hands its change to review; Codex sends it back with a note to cover the 429 retry path, then approves the fix, and the task lands as merged pull request #482. Each step is added to the event log.";

/** The room at work: the board with everyone on it, and the three beats it
 * plays, each caption lit while its beat is on the board. */
export function RoomSection() {
  const reduced = usePrefersReducedMotion();
  const [paused, setPaused] = useState(false);
  const [beat, setBeat] = useState<Beat>(0);
  const [near, setNear] = useState(false);
  const art = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = art.current;
    if (!el || typeof IntersectionObserver === "undefined") {
      setNear(true);
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        io.disconnect();
        setNear(true);
      },
      { rootMargin: "100% 0px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);
  const live = !reduced && !paused;
  return (
    <section className="room" aria-labelledby="room-title">
      <div className="room-head">
        <p className="eyebrow">The board</p>
        <h2 id="room-title">
          <span>Every task has one owner.</span>
          <span>Overlapping work waits.</span>
        </h2>
        <p className="lede">
          Claude Code, Codex, Cursor, Grok, Muse and Antigravity pull from the same board. Follow
          one task as it is claimed, reviewed and merged.
        </p>
      </div>
      <div className="room-stage">
        <div className="room-stage__art" ref={art} role="img" aria-label={LABEL}>
          {near ? (
            <Suspense fallback={null}>
              <RoomBoard paused={paused} onBeat={setBeat} />
            </Suspense>
          ) : null}
        </div>
        {reduced ? null : (
          <MediaToggle paused={paused} onToggle={() => setPaused((p) => !p)} noun="room animation" className="room-toggle" />
        )}
      </div>
      <ol className="room-steps">
        {STEPS.map((s, i) => (
          <li key={s.n} className="room-step" data-active={(live && beat === i + 1) || undefined}>
            <p className="room-step__n">{s.n}</p>
            <h3>{s.title}</h3>
            <p>{s.body}</p>
          </li>
        ))}
      </ol>
    </section>
  );
}
