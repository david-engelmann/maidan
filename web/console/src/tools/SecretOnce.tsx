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
  const [seen, setSeen] = useState(secret);
  const box = useRef<HTMLDivElement>(null);
  // During render, so a replacement secret never paints one frame in the clear
  // while the previous one was revealed.
  const view = secretView(copied, shown, secret, seen);
  if (view.seen !== seen) {
    setSeen(view.seen);
    setCopied(view.copied);
    setShown(view.shown);
  }
  useEffect(() => {
    box.current?.focus();
  }, [secret]);
  useEffect(() => {
    if (view.copied !== "yes") return;
    const timer = setTimeout(() => setCopied("no"), COPIED_FOR_MS);
    return () => clearTimeout(timer);
  }, [view.copied, secret]);
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
      <code className="secret-value" aria-label={view.shown ? "The secret" : "The secret, hidden"}>{view.shown ? secret : "•".repeat(Math.min(secret.length, 32))}</code>
      <div className="row-form">
        <button type="button" className="button steel" onClick={() => void copy()}>{view.copied === "yes" ? "Copied" : "Copy"}</button>
        <button type="button" className="button ghost" onClick={() => setShown((s) => !s)}>{view.shown ? "Hide" : "Show"}</button>
        <button type="button" className="button ghost" onClick={onDone}>I have it</button>
      </div>
      {view.copied === "yes" ? <span className="visually-hidden" role="status" aria-live="polite">Copied</span> : null}
      {view.copied === "failed" ? <p className="err small" role="alert">This browser would not copy. Show it and copy it by hand.</p> : null}
    </div>
  );
}

// Connect renders Add an agent, and Add an agent needs Copy. Keeping Copy
// here lets those two screens use each other without importing each other.
export function Copy({ text }: { text: string }) {
  const [said, setSaid] = useState<string | null>(null);
  const [seen, setSeen] = useState(text);
  // Same render-time reset as a secret card: a new snippet must not keep
  // "Copied" from the previous client tab.
  if (text !== seen) {
    setSeen(text);
    setSaid(null);
  }
  const label = text === seen ? said : null;
  useEffect(() => {
    if (label !== "Copied") return;
    const timer = setTimeout(() => setSaid(null), COPIED_FOR_MS);
    return () => clearTimeout(timer);
  }, [label, text]);
  return (
    <button
      type="button"
      className="button ghost"
      onClick={() => navigator.clipboard.writeText(text).then(() => setSaid("Copied"), () => setSaid("Copy failed"))}
    >
      {label ?? "Copy"}
    </button>
  );
}
