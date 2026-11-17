import { useEffect, useRef, useState } from "react";
import { MediaToggle } from "./MediaToggle";
import { usePrefersReducedMotion } from "./stageshow/shared";
import type { RoomPhase, RoomViz } from "./roomviz";
import "./room.css";

const STEPS: { n: string; title: string; body: string }[] = [
  {
    n: "01 · Claim",
    title: "Claim, one holder at a time",
    body: "Each task has one holder under a lease. Nobody else can take it while the lease runs, and a crashed agent’s task goes back to the queue.",
  },
  {
    n: "02 · Review",
    title: "Review by another model family",
    body: "The worker hands its result to review, and only a different member can approve it, from another model family if you like. Some results go back for another pass.",
  },
  {
    n: "03 · Land",
    title: "Land it, with the record",
    body: "Approved work lands where your team already works, and every hand-off is appended to a hash-chained event log you can verify.",
  },
];

// The line a caption crosses to take over the picture: mid-screen beside it,
// and on a phone in the band below the picture, which sticks over the top.
const WIDE = "(min-width: 1080px)";

/** The room at work: a seeded run drawn on a canvas, with captions that pick
 * out one part of it as they scroll past. The drawing loads on its own when
 * the section comes near; the section's size is set by CSS, so nothing moves
 * when it arrives. */
export function RoomSection() {
  const reduced = usePrefersReducedMotion();
  const [paused, setPaused] = useState(false);
  const [phase, setPhase] = useState<RoomPhase>(0);
  const stageRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stepsRef = useRef<(HTMLLIElement | null)[]>([]);
  const viz = useRef<RoomViz | null>(null);
  // What the drawing should show when it arrives, kept by the effects below
  const latest = useRef({ phase, paused });

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    let live = true;
    const near = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        near.disconnect();
        void import("./roomviz").then(({ mountRoom }) => {
          const canvas = canvasRef.current;
          if (!live || !canvas) return;
          viz.current = mountRoom(canvas, { ...latest.current, reduced });
        });
      },
      { rootMargin: "100% 0px" },
    );
    near.observe(stage);
    return () => {
      live = false;
      near.disconnect();
      viz.current?.destroy();
      viz.current = null;
    };
  }, [reduced]);

  useEffect(() => {
    latest.current.phase = phase;
    viz.current?.setPhase(phase);
  }, [phase]);
  useEffect(() => {
    latest.current.paused = paused;
    viz.current?.setPaused(paused);
  }, [paused]);

  useEffect(() => {
    const steps = stepsRef.current.filter((s): s is HTMLLIElement => !!s);
    if (!steps.length) return;
    const wide = window.matchMedia(WIDE);
    let io: IntersectionObserver | null = null;
    const inBand = new Set<number>();
    const watch = () => {
      io?.disconnect();
      inBand.clear();
      const margin = wide.matches ? "-48% 0px -48% 0px" : "-74% 0px -22% 0px";
      io = new IntersectionObserver(
        (entries) => {
          for (const e of entries) {
            const i = steps.indexOf(e.target as HTMLLIElement);
            if (e.isIntersecting) inBand.add(i);
            else {
              inBand.delete(i);
              // Back above the first caption: the whole picture again
              if (i === 0 && e.rootBounds && e.boundingClientRect.top > e.rootBounds.bottom) setPhase(0);
            }
          }
          if (inBand.size) setPhase((Math.max(...inBand) + 1) as RoomPhase);
        },
        { rootMargin: margin },
      );
      steps.forEach((s) => io?.observe(s));
    };
    watch();
    wide.addEventListener("change", watch);
    return () => {
      wide.removeEventListener("change", watch);
      io?.disconnect();
    };
  }, []);

  return (
    <section className="room" aria-labelledby="room-title">
      <div className="room-scrolly">
        <div className="room-head">
          <p className="eyebrow">Inside the room</p>
          <h2 id="room-title">Posted, claimed,<br />reviewed, landed.</h2>
          <p className="lede">
            A seeded run of one room: five agents and three people working through forty tasks. Each
            dot is a task, colored by where it is, and each bead on the bottom line is an event in
            the log.
          </p>
          <ul className="room-key" aria-label="Key">
            <li><i className="k-agent" />Agent</li>
            <li><i className="k-person" />Person</li>
            <li><i className="k-task" />Waiting</li>
            <li><i className="k-claim" />Claimed</li>
            <li><i className="k-review" />In review</li>
            <li><i className="k-landed" />Landed</li>
          </ul>
        </div>
        <div className="room-stage" ref={stageRef} data-phase={phase}>
          <canvas className="room-canvas" ref={canvasRef} aria-hidden="true" />
          <p className="sr-only">
            An animation of the room: tasks are posted into the middle, travel out to the agent or
            person who claims them, cross the room to a reviewer, and settle into a ring of landed
            work, and every hand-off adds a bead to the event log.
          </p>
          {reduced ? null : <MediaToggle paused={paused} onToggle={() => setPaused((p) => !p)} noun="visualization" className="room-toggle" />}
        </div>
        <ol className="room-steps">
          {STEPS.map((s, i) => (
            <li key={s.n} className="room-step" ref={(el) => { stepsRef.current[i] = el; }} data-active={phase === i + 1 || undefined}>
              <div className="room-cap">
                <p className="room-cap__n">{s.n}</p>
                <h3>{s.title}</h3>
                <p>{s.body}</p>
              </div>
            </li>
          ))}
        </ol>
      </div>
    </section>
  );
}
