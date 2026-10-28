import { AnimatePresence, LayoutGroup, motion, useReducedMotion } from "motion/react";
import { useEffect, useReducer, useRef, useState } from "react";
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

const spring = { type: "spring" as const, stiffness: 420, damping: 34, mass: 0.7 };

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

export function Board() {
  const reduce = useReducedMotion();
  const [model, dispatch] = useReducer(stepBoard, {
    cards: reduce ? settled() : opening(),
    note: "",
    reject: null as string | null,
  });
  const [who, setWho] = useState("north");
  const [lane, setLane] = useState<Lane>(reduce ? "done" : "open");
  const [run, setRun] = useState(0);
  const stop = useRef(false);
  const { cards, note, reject } = model;

  useEffect(() => {
    if (reduce) return;
    let cancel = false;
    const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
    const play = async () => {
      await wait(700);
      if (cancel || stop.current) return;
      dispatch({ id: "keys", holder: "north" });
      setLane("progress");
      await wait(1100);
      if (cancel || stop.current) return;
      dispatch({ id: "migration", holder: "ada" });
      setLane("review");
      await wait(1100);
      if (cancel || stop.current) return;
      dispatch({ id: "publish", holder: "jun" });
      setLane("review");
      await wait(800);
      if (cancel || stop.current) return;
      dispatch({ id: "publish", holder: "jun" });
      await wait(700);
      if (cancel || stop.current) return;
      dispatch({ id: "publish", holder: "jun" });
      setLane("done");
    };
    void play();
    return () => {
      cancel = true;
    };
  }, [reduce, run]);

  function tap(id: string) {
    stop.current = true;
    const step = advance(cards, id, who);
    dispatch({ id, holder: who });
    if (step.ok && step.note) navigator.vibrate?.(8);
    const next = step.cards.find((c) => c.id === id);
    if (next) setLane(next.lane);
  }

  function replay() {
    stop.current = false;
    dispatch({ type: "reset", cards: reduce ? settled() : opening() });
    setLane(reduce ? "done" : "open");
    setRun((n) => n + 1);
  }

  return (
    <section className="board" aria-labelledby="floor-title">
      <div className="board-top">
        <h2 id="floor-title">Release lane</h2>
        <span className="live">
          <span className="dot" aria-hidden="true" />
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
                initial={reduce ? false : { opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ ...spring, delay: reduce ? 0 : 0.08 * i }}
              >
                <span className={`avatar ${p.kind} ${p.id}`}>{p.initials}</span>
                <span>{p.name}</span>
                {live ? <span className="pulse" aria-hidden="true" /> : null}
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
            {item.label}
          </button>
        ))}
      </div>
      <LayoutGroup>
        <div className="lanes">
          {lanes.map((item) => (
            <section
              key={item.id}
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
                      reduce={!!reduce}
                      onTap={() => tap(c.id)}
                    />
                  ))}
              </AnimatePresence>
            </section>
          ))}
        </div>
      </LayoutGroup>
      <p className="hint-line" aria-live="polite">
        {note || "Choose someone, then tap a thread."}
      </p>
    </section>
  );
}

function CardView({
  card,
  reject,
  reduce,
  onTap,
}: {
  card: Card;
  reject: boolean;
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
      layout
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
      initial={reduce ? false : { opacity: 0, y: 16, scale: 0.98 }}
      animate={
        reject
          ? { opacity: 1, x: [0, -10, 9, -5, 0], scale: 1 }
          : { opacity: 1, y: 0, x: 0, scale: 1 }
      }
      exit={reduce ? { opacity: 0 } : { opacity: 0, y: -12, scale: 0.96 }}
      whileTap={reduce ? undefined : { scale: 0.975 }}
      transition={reduce ? { duration: 0 } : spring}
    >
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
            initial={reduce ? false : { scale: 1.35, rotate: -12, opacity: 0 }}
            animate={{ scale: 1, rotate: 0, opacity: 1 }}
            transition={reduce ? { duration: 0 } : spring}
          >
            Approved
          </motion.span>
        ) : null}
        {card.lane === "review" && !card.stamped ? <span className="needs">Needs you</span> : null}
        {card.evidence ? <span className="evidence">Evidence · 3 checks</span> : null}
      </span>
    </motion.button>
  );
}
