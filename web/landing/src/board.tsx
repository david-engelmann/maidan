import { AnimatePresence, LayoutGroup, motion, useReducedMotion } from "motion/react";
import { useEffect, useLayoutEffect, useReducer, useRef, useState } from "react";
import {
  advance,
  lanes,
  opening,
  people,
  person,
  settled,
  type Card,
  type Lane,
} from "./model";

const spring = { type: "spring" as const, stiffness: 460, damping: 32, mass: 0.62 };
const short: Record<Lane, string> = {
  open: "Open",
  progress: "Live",
  review: "Review",
  done: "Done",
};

type BoardState = { cards: Card[]; note: string; reject: string | null };
type BoardAction = { id: string; holder: string } | { type: "reset"; cards: Card[] };

function stepBoard(state: BoardState, action: BoardAction): BoardState {
  if ("type" in action) return { cards: action.cards, note: "", reject: null };
  const step = advance(state.cards, action.id, action.holder);
  return {
    cards: step.cards,
    note: step.note,
    reject: step.ok ? null : action.id,
  };
}

function useNarrow() {
  const [narrow, setNarrow] = useState(() => window.matchMedia("(max-width: 959px)").matches);
  useEffect(() => {
    const query = window.matchMedia("(max-width: 959px)");
    const onChange = () => setNarrow(query.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return narrow;
}

export function Board() {
  const reduce = useReducedMotion() === true;
  const [model, dispatch] = useReducer(stepBoard, {
    cards: reduce ? settled() : opening(),
    note: "",
    reject: null as string | null,
  });
  const [who, setWho] = useState("north");
  const [lane, setLane] = useState<Lane>("progress");
  const [run, setRun] = useState(0);
  const [flash, setFlash] = useState<string | null>(null);
  const narrow = useNarrow();
  const stop = useRef(false);
  const cardsRef = useRef(model.cards);
  const frameRef = useRef<HTMLDivElement>(null);
  const lock = useRef(false);
  const { cards, note, reject } = model;

  useEffect(() => {
    cardsRef.current = cards;
  }, [cards]);

  useEffect(() => {
    if (!flash) return;
    const id = window.setTimeout(() => setFlash(null), 780);
    return () => window.clearTimeout(id);
  }, [flash]);

  useLayoutEffect(() => {
    const frame = frameRef.current;
    if (!frame || !narrow) return;
    const node = frame.querySelector<HTMLElement>(`[data-lane="${lane}"]`);
    if (!node) return;
    lock.current = true;
    frame.scrollTo({ left: node.offsetLeft, behavior: reduce ? "auto" : "smooth" });
    const id = window.setTimeout(() => {
      lock.current = false;
    }, reduce ? 0 : 520);
    return () => window.clearTimeout(id);
  }, [lane, narrow, reduce]);

  useEffect(() => {
    if (reduce) return;
    let cancel = false;
    const timers: number[] = [];
    const wait = (ms: number) =>
      new Promise<void>((resolve) => {
        timers.push(window.setTimeout(resolve, ms));
      });
    const act = (id: string, holder: string) => {
      const step = advance(cardsRef.current, id, holder);
      dispatch({ id, holder });
      if (step.ok) setFlash(id);
    };
    const play = async () => {
      await wait(2200);
      if (cancel || stop.current) return;
      act("keys", "north");
      await wait(1500);
      if (cancel || stop.current) return;
      setLane("review");
      await wait(700);
      if (cancel || stop.current) return;
      act("migration", "ada");
      await wait(1400);
      if (cancel || stop.current) return;
      setLane("progress");
      await wait(500);
      if (cancel || stop.current) return;
      act("publish", "jun");
      await wait(900);
      if (cancel || stop.current) return;
      setLane("review");
      await wait(800);
      if (cancel || stop.current) return;
      act("publish", "jun");
      await wait(900);
      if (cancel || stop.current) return;
      act("publish", "jun");
      setLane("done");
    };
    void play();
    return () => {
      cancel = true;
      timers.forEach((id) => window.clearTimeout(id));
    };
  }, [reduce, run]);

  function tap(id: string) {
    stop.current = true;
    const step = advance(cards, id, who);
    dispatch({ id, holder: who });
    if (!step.ok) return;
    if (step.note) navigator.vibrate?.(12);
    setFlash(id);
    const next = step.cards.find((card) => card.id === id);
    if (next && next.lane !== lane) {
      window.setTimeout(() => setLane(next.lane), reduce ? 0 : 520);
    }
  }

  function replay() {
    stop.current = false;
    dispatch({ type: "reset", cards: reduce ? settled() : opening() });
    setLane("progress");
    setRun((n) => n + 1);
  }

  function onFrameScroll() {
    const frame = frameRef.current;
    if (!frame || !narrow || lock.current) return;
    const width = frame.clientWidth;
    if (!width) return;
    const index = Math.max(0, Math.min(lanes.length - 1, Math.round(frame.scrollLeft / width)));
    const next = lanes[index]?.id;
    if (next && next !== lane) setLane(next);
  }

  return (
    <section className="board" aria-labelledby="floor-title">
      <div className="board-top">
        <h2 id="floor-title">Release lane</h2>
        <span className="live">
          <span className="live-dot" aria-hidden="true" />
          Live
        </span>
        <button type="button" className="replay" onClick={replay}>
          Replay
        </button>
      </div>
      <ul className="presence" aria-label="Who is on the lane">
        {people.map((p, i) => {
          const live = cards.some(
            (c) => c.holder === p.id && (c.lane === "progress" || c.lane === "review"),
          );
          return (
            <li key={p.id}>
              <motion.button
                type="button"
                className={who === p.id ? "who is-on" : "who"}
                aria-pressed={who === p.id}
                onClick={() => setWho(p.id)}
                whileTap={reduce ? undefined : { scale: 0.94 }}
                initial={reduce ? false : { opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ ...spring, delay: reduce ? 0 : 0.05 * i }}
              >
                <span className={`avatar ${p.kind} ${p.id}`}>{p.initials}</span>
                <span>{p.name}</span>
                {live ? <span className="live-dot" aria-hidden="true" /> : null}
              </motion.button>
            </li>
          );
        })}
      </ul>
      <div className="segments" role="tablist" aria-label="Lanes">
        {lanes.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={lane === item.id}
            onClick={() => setLane(item.id)}
          >
            {lane === item.id ? (
              <motion.span className="pill" layoutId="lane-pill" transition={spring} />
            ) : null}
            <span className="seg-label">{short[item.id]}</span>
          </button>
        ))}
      </div>
      <div className="frame" ref={frameRef} onScroll={onFrameScroll}>
        <LayoutGroup>
          <div className="track">
            {lanes.map((item) => (
              <section
                key={item.id}
                data-lane={item.id}
                className={lane === item.id ? "lane is-current" : "lane"}
                aria-labelledby={`lane-${item.id}`}
              >
                <h3 id={`lane-${item.id}`}>{item.label}</h3>
                <AnimatePresence mode="popLayout">
                  {cards
                    .filter((c) => c.lane === item.id)
                    .map((c) => (
                      <CardView
                        key={c.id}
                        card={c}
                        reject={reject === c.id}
                        flash={flash === c.id}
                        reduce={reduce}
                        onTap={() => tap(c.id)}
                      />
                    ))}
                </AnimatePresence>
                {cards.every((c) => c.lane !== item.id) ? (
                  <p className="clear">Nothing here</p>
                ) : null}
              </section>
            ))}
          </div>
        </LayoutGroup>
      </div>
      <p className="hint-line" aria-live="polite">
        {note || "Choose someone, then tap a thread."}
      </p>
    </section>
  );
}

function CardView({
  card,
  reject,
  flash,
  reduce,
  onTap,
}: {
  card: Card;
  reject: boolean;
  flash: boolean;
  reduce: boolean;
  onTap: () => void;
}) {
  const holder = person(card.holder);
  const label = [
    card.title,
    lanes.find((l) => l.id === card.lane)?.label,
    holder ? `${holder.name}, ${holder.kind}` : "unclaimed",
    card.stamped ? "approved" : "",
    card.evidence ? "evidence attached" : "",
  ]
    .filter(Boolean)
    .join(", ");
  return (
    <motion.button
      type="button"
      layoutId={card.id}
      className={[
        "ticket",
        card.lane === "review" && !card.stamped ? "is-block" : "",
        card.stamped ? "is-stamped" : "",
        card.evidence ? "is-done" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      aria-label={label}
      onClick={onTap}
      initial={reduce ? false : { opacity: 0, scale: 0.96 }}
      animate={
        reject && !reduce
          ? { opacity: 1, x: [0, -8, 7, -3, 0], scale: 1 }
          : { opacity: 1, x: 0, scale: 1 }
      }
      exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.96 }}
      whileTap={reduce ? undefined : { scale: 0.97 }}
      transition={reduce ? { duration: 0 } : { ...spring, layout: spring }}
    >
      <motion.span
        className="ticket-face"
        animate={flash && !reduce ? { scale: [1, 1.035, 1] } : { scale: 1 }}
        transition={reduce ? { duration: 0 } : spring}
      >
        {flash && !reduce ? (
          <motion.span
            className="sheen"
            initial={{ x: "-160%" }}
            animate={{ x: "260%" }}
            transition={{ duration: 0.7, ease: [0.22, 1, 0.36, 1] }}
          />
        ) : null}
        <span className="ticket-title">{card.title}</span>
        <span className="meta">
          {holder ? (
            <>
              <span className={`avatar ${holder.kind} ${holder.id}`} aria-hidden="true">
                {holder.initials}
              </span>
              <span className="who-name">{holder.name}</span>
              <span className="kind">{holder.kind === "agent" ? "Agent" : "Human"}</span>
            </>
          ) : (
            <span className="unclaimed">Unclaimed</span>
          )}
          {card.stamped ? (
            <motion.span
              className="stamp"
              initial={reduce ? false : { scale: 0.7, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              transition={reduce ? { duration: 0 } : spring}
            >
              Approved
            </motion.span>
          ) : null}
          {card.lane === "review" && !card.stamped ? <span className="needs">Needs you</span> : null}
          {card.evidence ? <span className="evidence">Evidence · 3 checks</span> : null}
        </span>
      </motion.span>
    </motion.button>
  );
}
