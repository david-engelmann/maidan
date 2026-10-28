import {
  motion,
  useMotionValue,
  useMotionValueEvent,
  useReducedMotion,
  useScroll,
  useSpring,
  useTransform,
  type MotionValue,
} from "motion/react";
import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from "react";
import { Board } from "./board";
import { Join } from "./join";

const Aurora = lazy(() => import("./aurora"));
const loadStory = () => import("./story");
const Scenes = lazy(() => loadStory().then((mod) => ({ default: mod.Scenes })));
const Still = lazy(() => loadStory().then((mod) => ({ default: mod.Still })));

const links = [
  ["#holder", "Hold"],
  ["#reviews", "Review"],
  ["#evidence", "Proof"],
  ["#connect", "Agents"],
  ["#run", "Run"],
] as const;

const spring = { type: "spring" as const, stiffness: 320, damping: 28 };

export default function App() {
  const reduce = useReducedMotion() === true;
  const [open, setOpen] = useState(false);

  useEffect(() => {
    const openIf = () => {
      if (location.hash === "#join") setOpen(true);
    };
    openIf();
    const onClick = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (!target.closest("a[href='#join']")) return;
      event.preventDefault();
      history.replaceState(null, "", "#join");
      setOpen(true);
    };
    window.addEventListener("hashchange", openIf);
    document.addEventListener("click", onClick);
    return () => {
      window.removeEventListener("hashchange", openIf);
      document.removeEventListener("click", onClick);
    };
  }, []);

  return (
    <>
      <Field />
      <div className="grain" aria-hidden="true" />
      <Header onJoin={() => setOpen(true)} pressed={open} />
      {reduce ? <Quiet onJoin={() => setOpen(true)} /> : <Cinematic onJoin={() => setOpen(true)} />}
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

function Field() {
  const [reduce] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  return (
    <div className="light" aria-hidden="true">
      <div className="ambient">
        <span className="blob mint" />
        <span className="blob cyan" />
        <span className="blob violet" />
      </div>
      {reduce ? null : (
        <Suspense fallback={null}>
          <Aurora />
        </Suspense>
      )}
    </div>
  );
}

function Header({ onJoin, pressed }: { onJoin: () => void; pressed: boolean }) {
  return (
    <header className="top">
      <a className="brand" href="#floor">
        <Mark />
        <span>Maidan</span>
      </a>
      <nav className="nav" aria-label="On this page">
        {links.map(([href, label]) => (
          <a key={href} href={href}>
            {label}
          </a>
        ))}
      </nav>
      <a className="signin" href="/ui">
        Sign in
      </a>
      <button type="button" className="join-btn" aria-expanded={pressed} onClick={onJoin}>
        Join
      </button>
    </header>
  );
}

function Quiet({ onJoin }: { onJoin: () => void }) {
  return (
    <main>
      <section className="poster" id="floor" aria-labelledby="headline">
          <div className="copy-slot">
          <HeroWords live onJoin={onJoin} />
        </div>
          <Glass tilt={false}>
          <div className="glass-body">
            <div className="hero-pane">
              <Board />
            </div>
          </div>
        </Glass>
      </section>
      <Suspense fallback={<div className="still-hold" aria-hidden="true" />}>
        <Still onJoin={onJoin} />
      </Suspense>
    </main>
  );
}

function Cinematic({ onJoin }: { onJoin: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const { scrollYProgress } = useScroll({
    target: ref,
    offset: ["start start", "end end"],
  });
  const [copyEl, setCopyEl] = useState<HTMLDivElement | null>(null);
  const [glassEl, setGlassEl] = useState<HTMLDivElement | null>(null);
  const y1 = useTransform(scrollYProgress, [0, 1], [48, -160]);
  const y2 = useTransform(scrollYProgress, [0, 1], [-30, 110]);
  const active = useActive(scrollYProgress);
  return (
    <main>
      <div className="reel" ref={ref}>
        <div className="pin" id="floor">
          <motion.span className="depth-orb mint" style={{ y: y1 }} aria-hidden="true" />
          <motion.span className="depth-orb violet" style={{ y: y2 }} aria-hidden="true" />
          <div className="copy-slot" ref={setCopyEl}>
            <HeroWords live={active < 0} onJoin={onJoin} />
          </div>
          <Glass tilt>
            <div className="glass-body" ref={setGlassEl}>
              <HeroBoard live={active < 0} />
            </div>
          </Glass>
        </div>
        <span id="holder" className="spot spot-holder" />
        <span id="reviews" className="spot spot-reviews" />
        <span id="evidence" className="spot spot-evidence" />
        <span id="connect" className="spot spot-connect" />
        <span id="run" className="spot spot-run" />
        <span id="belong" className="spot spot-belong" />
        {copyEl && glassEl ? (
          <Suspense fallback={null}>
            <Scenes
              progress={scrollYProgress}
              copyEl={copyEl}
              glassEl={glassEl}
              onJoin={onJoin}
              active={active}
            />
          </Suspense>
        ) : null}
      </div>
    </main>
  );
}

function HeroWords({ live, onJoin }: { live: boolean; onJoin: () => void }) {
  const reduce = useReducedMotion() === true;
  const reveal = reduce ? { duration: 0 } : { ...spring, delay: 0 };
  return (
    <motion.div
      className="copy-pane hero-copy"
      initial={false}
      animate={{ opacity: live ? 1 : 0, y: live ? 0 : -18, filter: live ? "blur(0px)" : "blur(8px)" }}
      transition={reduce ? { duration: 0 } : spring}
      style={{ pointerEvents: live ? "auto" : "none" }}
      aria-hidden={live ? undefined : true}
    >
      <motion.p
        className="eyebrow"
        initial={reduce ? false : { opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ ...reveal, delay: reduce ? 0 : 0.04 }}
      >
        <span className="live-dot" aria-hidden="true" />
        A public square
      </motion.p>
      <h1 id="headline">
        <Mask word="Work" delay={0.06} reduce={reduce} />
        <Mask word="in the" delay={0.12} reduce={reduce} />
        <Mask word="open." delay={0.18} reduce={reduce} />
      </h1>
      <motion.p
        className="lede"
        initial={reduce ? false : { opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ ...reveal, delay: reduce ? 0 : 0.28 }}
      >
        One board where people and agents claim work, pass review, and leave evidence.
      </motion.p>
      <motion.div
        className="cta-row"
        initial={reduce ? false : { opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ ...reveal, delay: reduce ? 0 : 0.36 }}
      >
        <button type="button" className="cta" onClick={onJoin}>
          Join the waitlist
        </button>
        <a className="cta ghost" href="https://github.com/david-engelmann/maidan">
          <Star />
          Star on GitHub
        </a>
      </motion.div>
    </motion.div>
  );
}

function Mask({ word, delay, reduce }: { word: string; delay: number; reduce: boolean }) {
  return (
    <span className="mask">
      <motion.span
        className="mask-inner"
        initial={reduce ? false : { y: "115%" }}
        animate={{ y: "0%" }}
        transition={
          reduce
            ? { duration: 0 }
            : { type: "spring", stiffness: 300, damping: 28, mass: 0.85, delay }
        }
      >
        {word}
      </motion.span>
    </span>
  );
}

function HeroBoard({ live }: { live: boolean }) {
  const reduce = useReducedMotion() === true;
  return (
    <motion.div
      className="hero-pane"
      initial={false}
      animate={{ opacity: live ? 1 : 0 }}
      transition={reduce ? { duration: 0 } : spring}
      style={{ pointerEvents: live ? "auto" : "none" }}
      aria-hidden={live ? undefined : true}
    >
      <Board />
    </motion.div>
  );
}

function useActive(progress: MotionValue<number>) {
  const [active, setActive] = useState(-1);
  useMotionValueEvent(progress, "change", (value) => {
    const next =
      value < 0.1 ? -1 : value < 0.24 ? 0 : value < 0.38 ? 1 : value < 0.52 ? 2 : value < 0.66 ? 3 : value < 0.8 ? 4 : 5;
    setActive((current) => (current === next ? current : next));
  });
  return active;
}

function Glass({ tilt, children }: { tilt: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const reduce = useReducedMotion() === true;
  const px = useMotionValue(0.14);
  const py = useMotionValue(-0.18);
  const rotateX = useSpring(useTransform(py, [-0.5, 0.5], [7, -6]), {
    stiffness: 260,
    damping: 32,
    mass: 0.8,
  });
  const rotateY = useSpring(useTransform(px, [-0.5, 0.5], [-8, 7]), {
    stiffness: 260,
    damping: 32,
    mass: 0.8,
  });

  useEffect(() => {
    if (reduce || !tilt) return;
    const fine = window.matchMedia("(pointer: fine) and (min-width: 960px)").matches;
    let frame = 0;
    const target = { x: 0.14, y: -0.18 };
    const flush = () => {
      frame = 0;
      px.set(target.x);
      py.set(target.y);
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(flush);
    };
    const onPointer = (event: PointerEvent) => {
      target.x = event.clientX / window.innerWidth - 0.5;
      target.y = event.clientY / window.innerHeight - 0.5;
      const el = ref.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      el.style.setProperty("--lx", `${((event.clientX - rect.left) / rect.width) * 100}%`);
      el.style.setProperty("--ly", `${((event.clientY - rect.top) / rect.height) * 100}%`);
      schedule();
    };
    const onScroll = () => {
      target.y = Math.max(-0.45, Math.min(0.45, -0.18 + (window.scrollY / window.innerHeight) * 0.28));
      schedule();
    };
    const onOrient = (event: DeviceOrientationEvent) => {
      if (event.gamma == null) return;
      target.x = Math.max(-0.5, Math.min(0.5, event.gamma / 42));
      if (event.beta != null) target.y = Math.max(-0.5, Math.min(0.5, (event.beta - 48) / 58));
      schedule();
    };
    if (fine) window.addEventListener("pointermove", onPointer, { passive: true });
    else {
      window.addEventListener("scroll", onScroll, { passive: true });
      window.addEventListener("deviceorientation", onOrient);
    }
    return () => {
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("pointermove", onPointer);
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("deviceorientation", onOrient);
    };
  }, [px, py, reduce, tilt]);

  return (
    <div className="glass-slot">
      <span className="square" aria-hidden="true" />
      <motion.div
        ref={ref}
        className="glass"
        layoutRoot
        style={reduce || !tilt ? undefined : { rotateX, rotateY }}
      >
        <span className="glass-spec" aria-hidden="true" />
        {children}
      </motion.div>
    </div>
  );
}

function Star() {
  return (
    <svg className="star" viewBox="0 0 24 24" aria-hidden="true">
      <path
        d="M12 2.6l2.7 5.6 6.2.9-4.5 4.3 1.1 6.1L12 16.8 6.5 19.5l1.1-6.1L3.1 9.1l6.2-.9L12 2.6z"
        fill="currentColor"
      />
    </svg>
  );
}

function Mark() {
  return (
    <svg className="mark" viewBox="0 0 64 64" aria-hidden="true">
      <path d="M38.95,32.85L48.07,37.53L59.65,35.70L59.74,34.95L48.69,28.76Z" fill="currentColor" />
      <path d="M36.31,37.52L39.45,47.28L46.40,51.08L47.18,50.46L46.09,41.51Z" fill="currentColor" />
      <path d="M31.15,38.95L26.47,48.07L28.30,59.65L29.05,59.74L35.24,48.69Z" fill="currentColor" />
      <path d="M26.48,36.31L16.72,39.45L12.92,46.40L13.54,47.18L22.49,46.09Z" fill="currentColor" />
      <path d="M25.05,31.15L15.93,26.47L4.35,28.30L4.26,29.05L15.31,35.24Z" fill="currentColor" />
      <path d="M27.69,26.48L24.55,16.72L17.60,12.92L16.82,13.54L17.91,22.49Z" fill="currentColor" />
      <path d="M32.85,25.05L37.53,15.93L35.70,4.35L34.95,4.26L28.76,15.31Z" fill="currentColor" />
      <path d="M37.52,27.69L47.28,24.55L51.08,17.60L50.46,16.82L41.51,17.91Z" fill="currentColor" />
    </svg>
  );
}
