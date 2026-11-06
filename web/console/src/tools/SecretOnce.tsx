// A secret the server shows once: a minted or rotated token, a command's
// signing secret. It lives only in the caller's state, is never logged or
// stored, and goes when the user says they have it or the panel closes.
import { useEffect, useRef, useState } from "react";

export const COPIED_FOR_MS = 2500;

export type SecretCopied = "no" | "yes" | "failed";

/** A new secret starts hidden and uncopied, even when the same card stays mounted. */
export function secretView(copied: SecretCopied, shown: boolean, secret: string, seen: string): { copied: SecretCopied; shown: boolean; seen: string } {
  if (secret !== seen) return { copied: "no", shown: false, seen: secret };
  return { copied, shown, seen };
}

export function SecretOnce({ title, secret, onDone }: { title: string; secret: string; onDone: () => void }) {
  const [copied, setCopied] = useState<SecretCopied>("no");
  const [shown, setShown] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const seen = useRef(secret);
  useEffect(() => {
    if (seen.current !== secret) {
      const next = secretView("yes", true, secret, seen.current);
      setCopied(next.copied);
      setShown(next.shown);
      seen.current = next.seen;
    }
    box.current?.focus();
  }, [secret]);
  useEffect(() => {
    if (copied !== "yes") return;
    const timer = setTimeout(() => setCopied("no"), COPIED_FOR_MS);
    return () => clearTimeout(timer);
  }, [copied, secret]);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(secret);
      setCopied("yes");
    } catch {
      setCopied("failed");
    }
  };
  return (
    <div className="secret-once" role="region" aria-label={title} tabIndex={-1} ref={box}>
      <b>{title}</b>
      <p className="small">Shown once. Copy it now: the server keeps only a hash, so it cannot show it again.</p>
      <code className="secret-value" aria-label={shown ? "The secret" : "The secret, hidden"}>{shown ? secret : "•".repeat(Math.min(secret.length, 32))}</code>
      <div className="row-form">
        <button type="button" className="button steel" onClick={() => void copy()}>{copied === "yes" ? "Copied" : "Copy"}</button>
        <button type="button" className="button ghost" onClick={() => setShown((s) => !s)}>{shown ? "Hide" : "Show"}</button>
        <button type="button" className="button ghost" onClick={onDone}>I have it</button>
      </div>
      {copied === "yes" ? <span className="visually-hidden" role="status" aria-live="polite">Copied</span> : null}
      {copied === "failed" ? <p className="err small" role="alert">This browser would not copy. Show it and copy it by hand.</p> : null}
    </div>
  );
}
