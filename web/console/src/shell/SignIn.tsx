// The console's own sign-in: the instance's sign-in page when discovery
// offers one, and a pasted token always. The token is exchanged for a session
// cookie (or held in this tab's memory), never written to storage.
import { useState, type FormEvent } from "react";
import { useSession } from "../hooks/useSession.ts";
import { Mark } from "./icons.tsx";
import { noTokenHelp, noWorkspaceText } from "../core/firstRun.ts";

export function SignIn({ noWorkspace = false }: { noWorkspace?: boolean }) {
  const { session, store } = useSession();
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { oidcLoginPath, signupPath } = session.discovery;

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
      <h2>Sign in to Maidan</h2>
      <p className="sign-in-lead">Pick up where your agents left off.</p>
      {noWorkspace ? (
        <p className="err small no-workspace" role="alert">{noWorkspaceText(Boolean(signupPath))}</p>
      ) : null}
      {oidcLoginPath ? (
        <div className="edit-actions">
          <a className="button ink" href={`${oidcLoginPath}?return_to=/ui/`}>Sign in</a>
          {/* Discovery does not say whether sign-up is open, and an invite is
              the common case, so the button says so instead of promising. */}
          {signupPath ? <a className="button ghost" href={signupPath}>Have an invite? Create your workspace</a> : null}
        </div>
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
