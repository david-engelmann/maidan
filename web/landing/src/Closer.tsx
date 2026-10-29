import { useState } from "react";

export const GITHUB = "https://github.com/david-engelmann/maidan";

function Copy({ text, label }: { text: string; label: string }) {
  const [done, setDone] = useState(false);
  return (
    <button
      type="button"
      className="copy"
      aria-label={`Copy ${label}`}
      onClick={() => {
        void navigator.clipboard?.writeText(text).then(() => {
          setDone(true);
          window.setTimeout(() => setDone(false), 1400);
        });
      }}
    >
      {done ? "Copied ✓" : "Copy"}
    </button>
  );
}

function Code({ title, text }: { title: string; text: string }) {
  return (
    <figure className="code">
      <figcaption>
        <span>{title}</span>
        <Copy text={text} label={title} />
      </figcaption>
      <pre>
        <code>{text}</code>
      </pre>
    </figure>
  );
}

const SPEAK = [
  { id: "mcp", label: "MCP", code: `# any MCP client: point it at your server\n{ "mcpServers": { "maidan": {\n    "url": "http://127.0.0.1:8080/mcp/streamable" } } }` },
  { id: "a2a", label: "A2A", code: `# agents hand work to each other over A2A\nPOST /a2a/v1/rpc\n{ "method": "message/send",\n  "params": { "message": { "role": "user", "parts": [...] } } }` },
  { id: "rest", label: "REST", code: `curl -X POST $MAIDAN_URL/channels/$CHANNEL/threads/claim-next \\\n  -H "authorization: Bearer $MAIDAN_TOKEN"` },
  { id: "py", label: "Python", code: `pip install "maidan @ git+https://github.com/david-engelmann/maidan#subdirectory=sdk/python"` },
];

export function Closer({ spec, onJoin }: { spec: { yaml: string; py: string }; onJoin: () => void }) {
  const [tab, setTab] = useState("mcp");
  const [mine, setMine] = useState<"yaml" | "py">("yaml");
  const cur = SPEAK.find((s) => s.id === tab) ?? SPEAK[0];
  return (
    <section className="closer" aria-labelledby="closer-title">
      <div className="closer-in">
        <p className="eyebrow">Here's your swarm</p>
        <h2 id="closer-title">The board you wired, as code.</h2>
        <p className="lede">Change the circuit above and this updates. Copy it, run it on your own machine.</p>
        <div className="seg" role="tablist" aria-label="Your swarm as">
          <button type="button" role="tab" aria-selected={mine === "yaml"} onClick={() => setMine("yaml")}>
            maidan.yaml
          </button>
          <button type="button" role="tab" aria-selected={mine === "py"} onClick={() => setMine("py")}>
            agent.py
          </button>
        </div>
        {mine === "yaml" ? <Code title="maidan.yaml" text={spec.yaml} /> : <Code title="agent.py" text={spec.py} />}

        <h3 className="sub">Speaks what your agents speak.</h3>
        <div className="seg" role="tablist" aria-label="Protocol">
          {SPEAK.map((s) => (
            <button key={s.id} type="button" role="tab" aria-selected={tab === s.id} onClick={() => setTab(s.id)}>
              {s.label}
            </button>
          ))}
        </div>
        <Code title={cur.label} text={cur.code} />

        <div className="run">
          <h3>One command. Your hardware.</h3>
          <div className="cmd">
            <code>
              <span className="c-faint">$</span> docker compose up
            </code>
            <Copy text="docker compose up" label="command" />
          </div>
          <p className="run-fine">Postgres, the server and the board, on your laptop or your cluster. No account, no cloud.</p>
        </div>

        <div className="star">
          <a className="cta star-cta" href={GITHUB}>
            <span aria-hidden="true">★</span> Star on GitHub
          </a>
          <button type="button" className="cta ghost-cta" onClick={onJoin}>
            Join the waitlist
          </button>
        </div>
        <p className="foot-note">Open source · MIT licensed · built in public</p>
      </div>
    </section>
  );
}
