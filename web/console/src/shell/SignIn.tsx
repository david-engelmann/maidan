// The console's own sign-in: the instance's sign-in page when discovery
// offers one, and a pasted token always. The token is exchanged for a session
// cookie (or held in this tab's memory), never written to storage.
import { useState, type FormEvent } from "react";
import { useSession } from "../hooks/useSession.ts";

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
      <h2>Sign in to Maidan</h2>
      {noWorkspace ? (
        <p className="err small no-workspace" role="alert">
          You signed in, but that identity isn't a member of any workspace on this server, so no session was made. Ask an admin to add you, or create a workspace.
        </p>
      ) : null}
      {oidcLoginPath ? (
        <div className="edit-actions">
          <a className="button ink" href={`${oidcLoginPath}?return_to=/ui/`}>Sign in</a>
          {signupPath ? <a className="button ghost" href={signupPath}>Create a workspace</a> : null}
        </div>
      ) : null}
      <form className="sign-in-token" onSubmit={(e) => void submit(e)}>
        <label htmlFor="sign-in-token">{oidcLoginPath ? "Or paste a token" : "Paste a token"}</label>
        <input id="sign-in-token" type="password" autoComplete="off" spellCheck={false} value={secret} onChange={(e) => setSecret(e.target.value)} />
        <button type="submit" className="button steel" disabled={busy || !secret.trim()}>{busy ? "Signing in…" : "Sign in with token"}</button>
      </form>
      {error || session.error ? <p className="err small" role="alert">{error ?? session.error}</p> : null}
    </div>
  );
}
