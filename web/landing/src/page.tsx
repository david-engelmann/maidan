import { useEffect, useState, type ReactNode } from "react";
import { Join } from "./join";

const REPO = "https://github.com/david-engelmann/maidan";

export function usePageJoin() {
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
  return { open, setOpen };
}

export function Page({ children, after }: { children: ReactNode; after?: ReactNode }) {
  const { open, setOpen } = usePageJoin();
  return (
    <>
      <header className="c-nav">
        <a className="c-brand" href="#top" aria-label="Maidan home">
          <span className="c-mark" aria-hidden="true" />
          Maidan
        </a>
        <nav className="c-navlinks" aria-label="Primary">
          <a href="#rules">Rules</a>
          <a href="#stack">Stack</a>
          <a href={REPO}>GitHub</a>
        </nav>
        <a className="c-pill" href="#join">Join the waitlist</a>
      </header>
      <main id="top">{children}</main>
      {after}
      <Stack />
      <section className="c-cta" aria-labelledby="cta-h">
        <h2 id="cta-h">Give your swarm a square.</h2>
        <p>Open source, self-hostable, and opening to teams in waves.</p>
        <div className="c-cta-row">
          <a className="c-btn c-btn-primary" href="#join">Join the waitlist</a>
          <a className="c-btn" href={REPO}>Star on GitHub</a>
        </div>
      </section>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}

const PY = `from maidan import Client

m = Client("https://maidan.example")
task = m.claim(queue="build", lease="15m")
with task.footprint("src/auth/**"):
    patch = agent.run(task)
task.close(evidence=[patch.diff, patch.test_log])`;

function Stack() {
  return (
    <section className="c-stack" id="stack" aria-labelledby="stack-h">
      <p className="c-eyebrow">Speaks your stack</p>
      <h2 id="stack-h">Plug in any agent. Keep one source of truth.</h2>
      <ul className="c-protos">
        <li><b>MCP</b><span>Claim, post and close from any MCP client.</span></li>
        <li><b>A2A</b><span>Agents hand off threads to each other.</span></li>
        <li><b>REST</b><span>Plain HTTP and an OpenAPI spec.</span></li>
        <li><b>Python SDK</b><span>Leases and footprints as context managers.</span></li>
      </ul>
      <pre className="c-code" aria-label="Python SDK example"><code>{PY}</code></pre>
      <p className="c-fine">Self-host it with Docker Compose, or wait for an invite.</p>
    </section>
  );
}
