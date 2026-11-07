// The front door's chooser. Signing in through the identity provider with no
// workspace id lands in the identity's most recent workspace; when it has
// more than one, this asks where to work. Continuing here closes it; another
// workspace signs in there. Only this identity's workspaces are listed.
import { useEffect, useRef, useState } from "react";
import { switchHref, type SwitchableWorkspace } from "../core/tools.ts";
import { useSession } from "../hooks/useSession.ts";
import { useToolsApi } from "../hooks/useTools.ts";

const FOCUSABLE = "button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex='0']";

export function Chooser({ onClose }: { onClose: () => void }) {
  const { session } = useSession();
  const api = useToolsApi();
  const [spaces, setSpaces] = useState<SwitchableWorkspace[] | null>(null);
  const box = useRef<HTMLDivElement>(null);
  // The shell passes a fresh onClose each render. A ref keeps the fetch and
  // the key trap from restarting, which would pull focus back to the first
  // control while someone is tabbing.
  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return () => {
      if (opener && opener.isConnected) opener.focus({ preventScroll: true });
    };
  }, []);

  useEffect(() => {
    let live = true;
    api.switchable().then(
      (w) => {
        if (live) setSpaces(w);
      },
      () => {
        if (live) closeRef.current();
      },
    );
    return () => {
      live = false;
    };
  }, [api]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        closeRef.current();
        return;
      }
      if (e.key !== "Tab") return;
      const root = box.current;
      if (!root) return;
      const focusable = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (!focusable.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      const inside = active instanceof Node && root.contains(active);
      if (!inside || (e.shiftKey && active === first) || (!e.shiftKey && active === last)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!spaces) return;
    if (spaces.length <= 1) {
      closeRef.current();
      return;
    }
    box.current?.querySelector<HTMLElement>("button, a")?.focus();
  }, [spaces]);

  if (!spaces || spaces.length <= 1) return null;
  const base = session.credentials.base;
  const login = session.discovery.oidcLoginPath ?? undefined;
  return (
    <div className="chooser-scrim" onMouseDown={(e) => e.target === e.currentTarget && closeRef.current()}>
      <div className="card chooser" role="dialog" aria-modal="true" aria-label="Choose a workspace" ref={box}>
        <h2>Choose a workspace</h2>
        <p className="muted small chooser-note">You are in your most recent workspace. You belong to {spaces.length}.</p>
        <ul className="tool-list">
          {spaces.map((w) => (
            <li key={w.workspace_id} tabIndex={-1} className="tool-row" data-id={w.workspace_id}>
              <div className="grow"><b>{w.name}</b></div>
              {w.current ? (
                <button type="button" className="button steel" onClick={() => closeRef.current()}>Continue here</button>
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
