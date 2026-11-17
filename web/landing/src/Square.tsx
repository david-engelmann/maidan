import { useEffect, useId, useRef, type CSSProperties } from "react";
import "./square.css";

// The square the page stands on. A maidan is open ground where people
// gather, so the hero's board stands on one at dusk: paving in perspective
// running to a horizon behind the board, one warm light along that horizon,
// and a few lights crossing the paving toward the board, agents in the accent
// and people in warm white, arriving in one place. The closing band stands on
// the same ground, so the page ends where it began.
//
// Everything here is decoration: aria-hidden, under the words, static but for
// the lights, which only move by transform and opacity and stop when the
// hero's pause control is pressed, under reduced motion, or off screen.

// The paving, drawn once in the ground's own units: x runs -200..200, y 0..100
// from the horizon down, and the vanishing point is (0, 0). The SVG slices
// from its top middle, so a slab keeps its proportions at any width. A row of
// slabs at depth z sits at y = 100 / z (z = 1 at the foot, one slab deep per
// step), and a joint between slabs runs straight to the vanishing point.
const STEP = 0.16;
const JOINT = 24;

function paving(): { minor: string; major: string } {
  let minor = "";
  let major = "";
  for (let k = 0; ; k++) {
    const y = 100 / (1 + k * STEP);
    if (y < 11) break;
    const row = `M-200 ${y.toFixed(2)}H200`;
    // Every fourth joint is a band between larger fields of slabs
    if (k % 4 === 0) major += row;
    else minor += row;
  }
  for (let j = -26; j <= 26; j++) {
    const x = j * JOINT;
    const from = 9;
    const joint = `M${((x * from) / 100).toFixed(2)} ${from}L${x} 100`;
    if (j % 4 === 0) major += joint;
    else minor += joint;
  }
  return { minor, major };
}

const PAVING = paving();

type Walker = { kind: "agent" | "person"; x: number; dur: number; delay: number };

// Where each light sets out across the paving, as a share of the ground's
// width, and how long it takes to reach the board: slow and staggered, so
// one or two are on the ground at a time.
const WALKERS: Walker[] = [
  { kind: "agent", x: 67, dur: 19, delay: -3 },
  { kind: "person", x: 33, dur: 23, delay: -15 },
  { kind: "agent", x: 82, dur: 21, delay: -10 },
];

export function Square({ variant, paused = false }: { variant: "hero" | "close"; paused?: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  const id = useId().replace(/:/g, "");

  // Off screen the lights stop: nothing there to see, so nothing to run.
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver(([e]) => {
      el.toggleAttribute("data-away", !e.isIntersecting);
    });
    io.observe(el.querySelector(".sq-ground") ?? el);
    return () => io.disconnect();
  }, []);

  return (
    <div ref={ref} className={`sq sq--${variant}`} data-paused={paused || undefined} aria-hidden="true">
      <div className="sq-sky" />
      <div className="sq-ground">
        <div className="sq-floor" />
        <svg className="sq-paving" viewBox="-200 0 400 100" preserveAspectRatio="xMidYMin slice" focusable="false">
          <defs>
            {/* The joints catch the low light near the horizon, go neutral
                nearer the viewer, and fade out at both ends */}
            <linearGradient id={`${id}-j`} gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="0" y2="100">
              <stop className="sq-s sq-s--far" offset="0.1" />
              <stop className="sq-s sq-s--lit" offset="0.24" />
              <stop className="sq-s sq-s--near" offset="0.5" />
              <stop className="sq-s sq-s--end" offset="1" />
            </linearGradient>
          </defs>
          <path className="sq-joints" d={PAVING.minor} stroke={`url(#${id}-j)`} vectorEffect="non-scaling-stroke" />
          <path className="sq-joints sq-joints--major" d={PAVING.major} stroke={`url(#${id}-j)`} vectorEffect="non-scaling-stroke" />
        </svg>
        {variant === "hero" ? (
          <div className="sq-walkers">
            {WALKERS.map((w, i) => (
              <i
                key={i}
                className={`sq-walker sq-walker--${w.kind}`}
                style={{ "--x": w.x, "--dur": `${w.dur}s`, "--delay": `${w.delay}s` } as CSSProperties}
              />
            ))}
          </div>
        ) : null}
      </div>
      <div className="sq-sun" />
      <div className="sq-horizon" />
    </div>
  );
}
