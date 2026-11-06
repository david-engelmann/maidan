// The console's own sign-in: the instance's sign-in page when discovery
// offers one, and a pasted token always. The token is exchanged for a session
// cookie (or held in this tab's memory), never written to storage.
// Creating a workspace posts its name with the sign-up; blank is filled in
// by the server after the provider returns the handle.
import { useEffect, useRef, useState, type FormEvent, type RefObject } from "react";
import { useSession } from "../hooks/useSession.ts";
import { Mark } from "./icons.tsx";
import { noTokenHelp, noWorkspaceText } from "../core/firstRun.ts";
import { isWorkspaceInvite, readSignupCheck, workspaceNameRefusal } from "../core/signup.ts";

export function SignIn({ noWorkspace = false }: { noWorkspace?: boolean }) {
  const { session, store } = useSession();
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { oidcLoginPath, signupPath, turnstileSiteKey } = session.discovery;
  const check = readSignupCheck(window.location.search);
  const showCreate = Boolean(signupPath) || check.active;
  // Only the page the server sent here is joining: a stray invite query on
  // the ordinary sign-in card is not a reason to hide the name.
  const joining = check.active && isWorkspaceInvite(check.invite);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const token = secret.trim();
    if (!token || busy) return;
    setBusy(true);
    setError(null);
    const out = await store.signInWithToken(token);
    setBusy(false);
    if (out.ok) setSecret("");
    else if (out.error) setError(out.error);
  };

  return (
    <div className="card sign-in">
      <Mark className="sign-in-mark" />
      <h2>{check.active ? (joining ? "Join a workspace" : "Create a workspace") : "Sign in to Maidan"}</h2>
      <p className="sign-in-lead">{check.active && !joining ? "Name it, then sign in with your identity provider." : "Pick up where your agents left off."}</p>
      {noWorkspace ? (
        <p className="err small no-workspace" role="alert">{noWorkspaceText(Boolean(signupPath))}</p>
      ) : null}
      {oidcLoginPath ? (
        <div className="edit-actions">
          <a className="button ink" href={`${oidcLoginPath}?return_to=/ui/`}>Sign in</a>
        </div>
      ) : null}
      {showCreate ? (
        <CreateWorkspace
          action={signupPath ?? "/auth/oidc/signup"}
          initialName={joining ? "" : check.name}
          invite={check.active ? check.invite : null}
          joining={joining}
          returnTo={check.returnTo}
          siteKey={turnstileSiteKey}
          primary={check.active}
        />
      ) : null}
      {signupPath ? (
        <p className="sign-in-waitlist">No invite yet? <a href="/">Join the waitlist</a>.</p>
      ) : null}
      <form className="sign-in-token" onSubmit={(e) => void submit(e)}>
        <label htmlFor="sign-in-token">{oidcLoginPath ? "Or paste a token" : "Paste a token"}</label>
        <input id="sign-in-token" type="password" autoComplete="off" spellCheck={false} value={secret} onChange={(e) => setSecret(e.target.value)} />
        <button type="submit" className="button ink" disabled={busy || !secret.trim()}>{busy ? "Signing in…" : "Sign in with token"}</button>
      </form>
      {error || session.error ? <p className="err small" role="alert">{error ?? session.error}</p> : null}
      <p className="sign-in-help">{noTokenHelp(Boolean(oidcLoginPath))}</p>
    </div>
  );
}

function CreateWorkspace({
  action,
  initialName,
  invite,
  joining,
  returnTo,
  siteKey,
  primary,
}: {
  action: string;
  initialName: string;
  invite: string | null;
  joining: boolean;
  returnTo: string;
  siteKey: string | null;
  primary: boolean;
}) {
  const [name, setName] = useState(initialName);
  const why = joining ? null : workspaceNameRefusal(name);
  const { slot, token, failed } = useSignupTurnstile(siteKey);
  const needsCheck = Boolean(siteKey);
  const blocked = Boolean(why) || (needsCheck && !token);

  return (
    <form
      className="sign-in-token"
      method="post"
      action={action}
      onSubmit={(e) => {
        if (blocked) e.preventDefault();
      }}
    >
      {joining ? null : (
        <>
          <label htmlFor="workspace-name">Workspace name</label>
          <input
            id="workspace-name"
            name="workspace_name"
            value={name}
            placeholder="Your team's name"
            autoComplete="organization"
            autoCapitalize="words"
            spellCheck={false}
            aria-describedby="workspace-name-hint"
            onChange={(e) => setName(e.target.value)}
          />
          <p id="workspace-name-hint" className="muted small">Leave it blank and it will be named after you.</p>
          {why ? <p className="err small" role="alert">{why}</p> : null}
        </>
      )}
      {invite ? <input type="hidden" name="invite" value={invite} /> : null}
      <input type="hidden" name="return_to" value={returnTo} />
      {needsCheck ? <input type="hidden" name="turnstile" value={token} /> : null}
      {needsCheck ? <div ref={slot} /> : null}
      {failed ? <p className="err small" role="alert">The check could not load. Refresh the page and try again.</p> : null}
      <button type="submit" className={primary ? "button ink" : "button ghost"} disabled={blocked}>
        {joining ? "Continue" : primary ? "Create your workspace" : "Have an invite? Create your workspace"}
      </button>
    </form>
  );
}

function useSignupTurnstile(siteKey: string | null): { slot: RefObject<HTMLDivElement | null>; token: string; failed: boolean } {
  const slot = useRef<HTMLDivElement>(null);
  const [token, setToken] = useState("");
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!siteKey) return;
    let gone = false;
    const run = async () => {
      try {
        await loadTurnstile();
      } catch {
        if (!gone) setFailed(true);
        return;
      }
      if (gone || !window.turnstile || !slot.current) {
        if (!gone) setFailed(true);
        return;
      }
      window.turnstile.render(slot.current, {
        sitekey: siteKey,
        action: "signup",
        callback: (value) => setToken(value),
        "expired-callback": () => setToken(""),
      });
    };
    void run();
    return () => {
      gone = true;
    };
  }, [siteKey]);
  return { slot, token, failed };
}

function loadTurnstile(): Promise<void> {
  if (window.turnstile) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const existing = document.querySelector<HTMLScriptElement>("script[data-turnstile='signup']");
    if (existing) {
      existing.addEventListener("load", () => resolve(), { once: true });
      existing.addEventListener("error", () => reject(new Error("turnstile")), { once: true });
      return;
    }
    const script = document.createElement("script");
    script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
    script.async = true;
    script.dataset.turnstile = "signup";
    script.onload = () => resolve();
    script.onerror = () => reject(new Error("turnstile"));
    document.head.appendChild(script);
  });
}
