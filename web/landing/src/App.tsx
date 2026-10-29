import { useReducedMotion } from "motion/react";
import { useCallback, useEffect, useState } from "react";
import { Circuit } from "./Circuit";
import { Closer, GITHUB } from "./Closer";
import { Hook } from "./Hook";
import { Join } from "./join";
import { FILM } from "./loop";
import { Stage } from "./Stage";

export default function App() {
  const [open, setOpen] = useState(false);
  const reduce = useReducedMotion() ?? false;
  const [spec, setSpec] = useState({ yaml: "", py: "" });
  useEffect(() => {
    const openIf = () => {
      if (location.hash === "#join") setOpen(true);
    };
    openIf();
    window.addEventListener("hashchange", openIf);
    return () => window.removeEventListener("hashchange", openIf);
  }, []);
  useEffect(() => {
    if (FILM) void import("./film").then((m) => m.playFilm());
  }, []);
  const join = () => setOpen(true);
  const onSpec = useCallback((yaml: string, py: string) => setSpec({ yaml, py }), []);
  return (
    <>
      <header className="top">
        <a className="brand" href="#top">
          <span className="brand-dot" aria-hidden="true" />
          Maidan
        </a>
        <span className="top-space" />
        <a className="top-link" href={GITHUB}>
          <span aria-hidden="true">★</span> Star
        </a>
        <button type="button" className="top-join" onClick={join}>
          Join
        </button>
      </header>
      <main id="top">
        <Hook reduce={reduce} />
        <Circuit reduce={reduce} onSpec={onSpec} />
        <Stage reduce={reduce} onJoin={join} />
        <Closer spec={spec} onJoin={join} />
      </main>
      <Join open={open} onOpenChange={setOpen} />
    </>
  );
}
