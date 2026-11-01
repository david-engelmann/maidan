// The front door's chooser. Signing in through the identity provider with no
// workspace id lands in the identity's most recent workspace; when it has
// more than one, this asks where to work. Continuing here closes it; another
// workspace signs in there. Only this identity's workspaces are listed.
import { useEffect, useRef, useState } from "react";
import { switchHref, type SwitchableWorkspace } from "../core/tools.ts";
import { useSession } from "../hooks/useSession.ts";
import { useToolsApi } from "../hooks/useTools.ts";

export function Chooser({ onClose }: { onClose: () => void }) {
  const { session } = useSession();
  const api = useToolsApi();
  const [spaces, setSpaces] = useState<SwitchableWorkspace[] | null>(null);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let live = true;
    void api.switchable().then((w) => live && setSpaces(w));
    return () => {
      live = false;
    };
  }, [api]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  useEffect(() => {
    if (spaces && spaces.length > 1) box.current?.querySelector<HTMLElement>("button, a")?.focus();
    if (spaces && spaces.length <= 1) onClose();
  }, [spaces, onClose]);
  if (!spaces || spaces.length <= 1) return null;
  const base = session.credentials.base;
  const login = session.discovery.oidcLoginPath ?? undefined;
  return (
    <div className="chooser-scrim">
      <div className="card chooser" role="dialog" aria-modal="true" aria-label="Choose a workspace" ref={box}>
        <h2>Choose a workspace</h2>
        <p className="muted small chooser-note">You are in your most recent workspace. You belong to {spaces.length}.</p>
        <ul className="tool-list">
          {spaces.map((w) => (
            <li key={w.workspace_id} className="tool-row" data-id={w.workspace_id}>
              <div className="grow"><b>{w.name}</b></div>
              {w.current ? (
                <button type="button" className="button steel" onClick={onClose}>Continue here</button>
              ) : (
                <a className="button ghost" href={switchHref(base, w.workspace_id, login)}>Switch</a>
              )}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
