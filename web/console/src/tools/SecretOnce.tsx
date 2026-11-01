// A secret the server shows once: a minted or rotated token, a command's
// signing secret. It lives only in the caller's state, is never logged or
// stored, and goes when the user says they have it or the panel closes.
import { useEffect, useRef, useState } from "react";

export function SecretOnce({ title, secret, onDone }: { title: string; secret: string; onDone: () => void }) {
  const [copied, setCopied] = useState<"no" | "yes" | "failed">("no");
  const [shown, setShown] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    box.current?.focus();
  }, []);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(secret);
      setCopied("yes");
    } catch {
      setCopied("failed");
    }
  };
  return (
    <div className="secret-once" role="alert" tabIndex={-1} ref={box}>
      <b>{title}</b>
      <p className="small">Shown once. Copy it now: the server keeps only a hash, so it cannot show it again.</p>
      <code className="secret-value" aria-label={shown ? "The secret" : "The secret, hidden"}>{shown ? secret : "•".repeat(Math.min(secret.length, 32))}</code>
      <div className="row-form">
        <button type="button" className="button steel" onClick={() => void copy()}>{copied === "yes" ? "Copied" : "Copy"}</button>
        <button type="button" className="button ghost" onClick={() => setShown((s) => !s)}>{shown ? "Hide" : "Show"}</button>
        <button type="button" className="button ghost" onClick={onDone}>I have it</button>
      </div>
      {copied === "failed" ? <p className="err small">This browser would not copy. Show it and copy it by hand.</p> : null}
    </div>
  );
}
