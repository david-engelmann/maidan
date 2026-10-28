import { motion, useMotionValue, useTransform, type MotionValue } from "motion/react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";

const scenes = [
  {
    id: "claims",
    kicker: "Hold",
    title: "One holder.",
    body: "A second claim waits.",
  },
  {
    id: "reviews",
    kicker: "Review",
    title: "It comes to you.",
    body: "When work blocks the lane, it is the lane.",
  },
  {
    id: "evidence",
    kicker: "Proof",
    title: "The evidence stays.",
    body: "Checks, the result, and the handoff.",
  },
  {
    id: "connect",
    kicker: "Connect",
    title: "From an agent.",
    body: "The same claim, over MCP or a few lines.",
  },
  {
    id: "ways",
    kicker: "Run",
    title: "Yours, or hosted.",
    body: "One open-source server. SQLite or Postgres.",
  },
] as const;

const fade = { duration: 0.45, ease: [0.22, 1, 0.36, 1] as const };

export function Scenes({
  progress,
  copyEl,
  glassEl,
  onJoin,
  active,
}: {
  progress: MotionValue<number>;
  copyEl: HTMLElement;
  glassEl: HTMLElement;
  onJoin: () => void;
  active: number;
}) {
  return (
    <>
      {createPortal(
        <div className="scene-layer">
          {scenes.map((scene, index) => (
            <CopyPane key={scene.id} index={index} active={active} scene={scene} />
          ))}
        </div>,
        copyEl,
      )}
      {createPortal(
        <div className="scene-layer">
          {scenes.map((scene, index) => (
            <GlassPane key={scene.id} index={index} active={active}>
              <Visual index={index} progress={progress} onJoin={onJoin} />
            </GlassPane>
          ))}
        </div>,
        glassEl,
      )}
    </>
  );
}

function CopyPane({
  index,
  active,
  scene,
}: {
  index: number;
  active: number;
  scene: (typeof scenes)[number];
}) {
  const on = active === index;
  return (
    <motion.div
      className="copy-pane"
      initial={false}
      animate={{ opacity: on ? 1 : 0, y: on ? 0 : 14 }}
      transition={fade}
      style={{ pointerEvents: on ? "auto" : "none" }}
      aria-hidden={!on}
    >
      <p className="kicker">{scene.kicker}</p>
      <h2 id={`${scene.id}-title`}>{scene.title}</h2>
      <p className="lede">{scene.body}</p>
    </motion.div>
  );
}

function GlassPane({
  index,
  active,
  children,
}: {
  index: number;
  active: number;
  children: ReactNode;
}) {
  const on = active === index;
  return (
    <motion.div
      className="glass-pane"
      initial={false}
      animate={{ opacity: on ? 1 : 0, y: on ? 0 : 16 }}
      transition={fade}
      style={{ pointerEvents: on ? "auto" : "none" }}
      aria-hidden={!on}
    >
      {children}
    </motion.div>
  );
}

function Visual({
  index,
  progress,
  onJoin,
}: {
  index: number;
  progress: MotionValue<number>;
  onJoin: () => void;
}) {
  if (index === 0) return <HoldVisual />;
  if (index === 1) return <ReviewVisual progress={progress} />;
  if (index === 2) return <ProofVisual progress={progress} />;
  if (index === 3) return <CodeVisual />;
  return <WaysVisual onJoin={onJoin} />;
}

function HoldVisual() {
  return (
    <div className="hold">
      <article className="ticket static">
        <span className="ticket-title">Rotate session keys</span>
        <span className="meta">
          <span className="avatar agent north">N</span>
          <span className="who-name">North</span>
          <span className="kind">Agent</span>
          <span className="stamp">Held</span>
        </span>
      </article>
      <div className="thread" aria-hidden="true">
        <span className="thread-line" />
        <span className="avatar agent sol">S</span>
        <span className="thread-note">Sol is waiting</span>
      </div>
    </div>
  );
}

function ReviewVisual({ progress }: { progress: MotionValue<number> }) {
  const local = useTransform(progress, [0.27, 0.42], [0, 1]);
  const needsOpacity = useTransform(local, [0.38, 0.62], [1, 0]);
  const stampOpacity = useTransform(local, [0.42, 0.7], [0, 1]);
  const stampScale = useTransform(local, [0.42, 0.74], [0.82, 1]);
  return (
    <article className="ticket static is-block">
      <span className="ticket-title">Question on the lease</span>
      <span className="meta">
        <span className="avatar human ada">A</span>
        <span className="who-name">Ada</span>
        <span className="kind">Human</span>
        <span className="swap">
          <motion.span className="needs" style={{ opacity: needsOpacity }}>
            Needs you
          </motion.span>
          <motion.span className="stamp" style={{ opacity: stampOpacity, scale: stampScale }}>
            Approved
          </motion.span>
        </span>
      </span>
    </article>
  );
}

const proof = [
  "Tests green on the release branch",
  "Migration applied, then read back",
  "Ada approved the diff",
];

function ProofVisual({ progress }: { progress: MotionValue<number> }) {
  const local = useTransform(progress, [0.44, 0.58], [0, 1]);
  return (
    <ol className="proof">
      {proof.map((line, index) => (
        <ProofRow key={line} local={local} index={index} text={line} />
      ))}
    </ol>
  );
}

function ProofRow({
  local,
  index,
  text,
}: {
  local: MotionValue<number>;
  index: number;
  text: string;
}) {
  const opacity = useTransform(local, [0.08 + index * 0.2, 0.28 + index * 0.2], [0, 1]);
  const y = useTransform(local, [0.08 + index * 0.2, 0.28 + index * 0.2], [12, 0]);
  return (
    <motion.li style={{ opacity, y }}>
      <span className="tick" aria-hidden="true" />
      {text}
    </motion.li>
  );
}

function CodeVisual() {
  return (
    <figure className="snippet">
      <figcaption>
        Python SDK. MCP exposes <span className="mono">claim_next_thread</span> the same way.
      </figcaption>
      <pre tabIndex={0}>
        <code>
          <span className="fn">claim</span> = client.
          <span className="fn">claim_next_thread</span>(channel_id){"\n"}
          <span className="kw">if</span> claim:{"\n"}
          {"    "}client.messages.<span className="fn">post</span>(claim.id,{" "}
          <span className="str">&quot;on it&quot;</span>){"\n"}
          {"    "}client.threads.<span className="fn">set_result</span>({"\n"}
          {"        "}claim.id, {"{"}
          <span className="str">&quot;ok&quot;</span>: <span className="kw">True</span>
          {"}"}, claim.claim_lease_id{"\n"}
          {"    "})
        </code>
      </pre>
    </figure>
  );
}

function WaysVisual({ onJoin }: { onJoin: () => void }) {
  return (
    <div className="ways">
      <a className="way" href="https://github.com/david-engelmann/maidan">
        <span className="way-kicker">Self-host</span>
        <span className="way-title">Your database. Your agents.</span>
      </a>
      <a
        className="way is-lit"
        href="#join"
        onClick={(event) => {
          event.preventDefault();
          onJoin();
        }}
      >
        <span className="way-kicker">Hosted</span>
        <span className="way-title">Ask for a place.</span>
      </a>
    </div>
  );
}

export function Still({ onJoin }: { onJoin: () => void }) {
  const progress = useMotionValue(1);
  return (
    <div className="still">
      {scenes.map((scene, index) => (
        <section key={scene.id} id={scene.id} className="still-block" aria-labelledby={`${scene.id}-title`}>
          <p className="kicker">{scene.kicker}</p>
          <h2 id={`${scene.id}-title`}>{scene.title}</h2>
          <p className="lede">{scene.body}</p>
          <div className="glass still-glass">
            <Visual index={index} progress={progress} onJoin={onJoin} />
          </div>
        </section>
      ))}
    </div>
  );
}
