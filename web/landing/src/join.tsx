import { motion, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState, type FormEvent } from "react";

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

type Field = "wl-email" | "wl-use" | "wl-consent";

export function Join() {
  const reduce = useReducedMotion();
  const email = useRef<HTMLInputElement>(null);
  const useCase = useRef<HTMLTextAreaElement>(null);
  const consent = useRef<HTMLInputElement>(null);
  const website = useRef<HTMLInputElement>(null);
  const slot = useRef<HTMLDivElement>(null);
  const widget = useRef<string | null>(null);
  const token = useRef("");
  const [errors, setErrors] = useState<Partial<Record<Field, string>>>({});
  const [status, setStatus] = useState("");
  const [bad, setBad] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);

  useEffect(() => {
    let gone = false;
    const start = async () => {
      let siteKey: string | null = null;
      try {
        const res = await fetch("/.well-known/maidan.json", { credentials: "omit" });
        if (res.ok) {
          const body = (await res.json()) as { auth?: { turnstile_site_key?: string } };
          siteKey = body.auth?.turnstile_site_key || null;
        }
      } catch {
        siteKey = null;
      }
      if (!siteKey || gone) return;
      await new Promise<void>((resolve, reject) => {
        const script = document.createElement("script");
        script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
        script.async = true;
        script.onload = () => resolve();
        script.onerror = () => reject(new Error("turnstile"));
        document.head.appendChild(script);
      }).catch(() => undefined);
      if (gone || !window.turnstile || !slot.current) return;
      slot.current.hidden = false;
      widget.current = window.turnstile.render(slot.current, {
        sitekey: siteKey,
        action: "waitlist",
        callback: (value) => {
          token.current = value;
        },
        "expired-callback": () => {
          token.current = "";
        },
      });
    };
    void start();
    return () => {
      gone = true;
    };
  }, []);

  function validate(): Field | null {
    const next: Partial<Record<Field, string>> = {};
    let first: Field | null = null;
    const emailValue = email.current?.value.trim() ?? "";
    const useValue = useCase.current?.value.trim() ?? "";
    if (!EMAIL.test(emailValue)) {
      next["wl-email"] = "Enter an email address like you@example.com.";
      first = "wl-email";
    }
    if (!useValue) {
      next["wl-use"] = "Add a line about what you would use Maidan for.";
      first ??= "wl-use";
    }
    if (!consent.current?.checked) {
      next["wl-consent"] = "Tick the box so we may email you your invite.";
      first ??= "wl-consent";
    }
    setErrors(next);
    return first;
  }

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    const invalid = validate();
    if (invalid) {
      document.getElementById(invalid)?.focus();
      setStatus("Check the highlighted fields.");
      setBad(true);
      return;
    }
    setBusy(true);
    setBad(false);
    setStatus("Sending…");
    let res: Response | null = null;
    try {
      res = await fetch("/waitlist", {
        method: "POST",
        credentials: "omit",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify({
          email: email.current?.value.trim(),
          use_case: useCase.current?.value.trim(),
          consent: true,
          website: website.current?.value ?? "",
          ...(widget.current !== null ? { turnstile_token: token.current } : {}),
        }),
      });
    } catch {
      res = null;
    }
    if (widget.current !== null && window.turnstile) {
      token.current = "";
      window.turnstile.reset(widget.current);
    }
    setBusy(false);
    if (res?.ok) {
      setDone(true);
      return;
    }
    setBad(true);
    if (res?.status === 429) {
      setStatus("Too many tries from this network. Wait a little and try again.");
    } else if (res?.status === 403) {
      setStatus("Complete the check above, then try again.");
    } else if (res?.status === 400) {
      setStatus("That didn't look right. Check your email and note, then try again.");
    } else {
      setStatus("We couldn't reach the server. Try again in a moment.");
    }
  }

  return (
    <section className="close" id="join" aria-labelledby="join-title">
      <motion.div
        className="close-copy"
        initial={reduce ? false : { opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, amount: 0.5 }}
        transition={reduce ? { duration: 0 } : { type: "spring", stiffness: 280, damping: 32 }}
      >
        <p className="kicker">The list</p>
        <h2 id="join-title">Take a place on the board.</h2>
        <p>We invite people in batches. One email when your place comes up.</p>
      </motion.div>
      <div className="card">
        {done ? (
          <div className="done" tabIndex={-1}>
            <h2>You&apos;re on the list.</h2>
            <p>We&apos;ll email you when your invite is ready. Nothing else, ever.</p>
          </div>
        ) : (
          <form id="waitlist-form" noValidate onSubmit={onSubmit}>
            <div className="field">
              <label htmlFor="wl-email">Work email</label>
              <input
                ref={email}
                id="wl-email"
                name="email"
                type="email"
                inputMode="email"
                autoComplete="email"
                maxLength={254}
                required
                aria-describedby="wl-email-err"
                aria-invalid={errors["wl-email"] ? true : undefined}
                placeholder="you@example.com"
                onInput={() => {
                  if (errors["wl-email"]) validate();
                }}
              />
              <p className="err" id="wl-email-err" hidden={!errors["wl-email"]}>
                {errors["wl-email"]}
              </p>
            </div>
            <div className="field">
              <label htmlFor="wl-use">What would you use Maidan for?</label>
              <textarea
                ref={useCase}
                id="wl-use"
                name="use_case"
                rows={3}
                maxLength={2000}
                required
                aria-describedby="wl-use-err"
                aria-invalid={errors["wl-use"] ? true : undefined}
                placeholder="e.g. a review swarm for our backend repo"
                onInput={() => {
                  if (errors["wl-use"]) validate();
                }}
              />
              <p className="err" id="wl-use-err" hidden={!errors["wl-use"]}>
                {errors["wl-use"]}
              </p>
            </div>
            <div className="hp" aria-hidden="true">
              <label htmlFor="wl-website">Leave this empty</label>
              <input
                ref={website}
                id="wl-website"
                name="website"
                type="text"
                tabIndex={-1}
                autoComplete="off"
              />
            </div>
            <div className="check">
              <input
                ref={consent}
                id="wl-consent"
                name="consent"
                type="checkbox"
                required
                aria-describedby="wl-consent-err"
                aria-invalid={errors["wl-consent"] ? true : undefined}
                onChange={() => {
                  if (errors["wl-consent"]) validate();
                }}
              />
              <label htmlFor="wl-consent">You may email me about my place on the list.</label>
            </div>
            <p className="err" id="wl-consent-err" hidden={!errors["wl-consent"]}>
              {errors["wl-consent"]}
            </p>
            <div className="turnstile" id="wl-turnstile" ref={slot} hidden />
            <button id="wl-submit" type="submit" aria-busy={busy || undefined} disabled={busy}>
              Request an invite
            </button>
            <p className={bad ? "status is-error" : "status"} role="status" aria-live="polite">
              {status}
            </p>
          </form>
        )}
      </div>
    </section>
  );
}
