// The public waitlist page. One small module, no dependencies: it checks
// the fields, posts JSON to /waitlist on this origin, and shows the answer.
// The server answers a repeat address exactly like a new one, so the page
// never learns, and never says, whether someone was already listed.

const form = document.getElementById("waitlist-form");
const email = document.getElementById("wl-email");
const useCase = document.getElementById("wl-use");
const consent = document.getElementById("wl-consent");
const website = document.getElementById("wl-website");
const submit = document.getElementById("wl-submit");
const status = document.getElementById("wl-status");
const done = document.getElementById("wl-done");

const slot = document.getElementById("wl-turnstile");

// Turnstile, when the instance runs it: discovery names the site key, and
// the widget script loads from Cloudflare's challenge origin, which this
// page's CSP allows only then. A token is good for one join, so the widget
// resets after every submit.
let widget = null;
let token = "";

async function startTurnstile() {
  let siteKey = null;
  try {
    const res = await fetch("/.well-known/maidan.json", { credentials: "omit" });
    if (res.ok) siteKey = ((await res.json()).auth || {}).turnstile_site_key || null;
  } catch {
    siteKey = null;
  }
  if (!siteKey) return;
  const script = document.createElement("script");
  script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
  script.async = true;
  script.onload = () => {
    slot.hidden = false;
    widget = window.turnstile.render(slot, {
      sitekey: siteKey,
      action: "waitlist",
      callback: (t) => { token = t; },
      "expired-callback": () => { token = ""; },
    });
  };
  document.head.appendChild(script);
}

startTurnstile();

const EMAIL_SHAPE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function setError(input, message) {
  const err = document.getElementById(`${input.id}-err`);
  if (message) {
    input.setAttribute("aria-invalid", "true");
    err.textContent = message;
    err.hidden = false;
  } else {
    input.removeAttribute("aria-invalid");
    err.textContent = "";
    err.hidden = true;
  }
}

function validate() {
  const checks = [
    [email, EMAIL_SHAPE.test(email.value.trim()) ? "" : "Enter an email address like you@company.com."],
    [useCase, useCase.value.trim() ? "" : "Add a line about what you would use Maidan for."],
    [consent, consent.checked ? "" : "Tick the box so we may email you your invite."],
  ];
  let first = null;
  for (const [input, message] of checks) {
    setError(input, message);
    if (message && !first) first = input;
  }
  return first;
}

function say(message, isError) {
  status.textContent = message;
  status.classList.toggle("is-error", isError);
}

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const invalid = validate();
  if (invalid) {
    invalid.focus();
    say("Check the highlighted fields.", true);
    return;
  }
  submit.setAttribute("aria-busy", "true");
  submit.disabled = true;
  say("Sending…", false);
  let res;
  try {
    res = await fetch("/waitlist", {
      method: "POST",
      credentials: "omit",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({
        email: email.value.trim(),
        use_case: useCase.value.trim(),
        consent: true,
        website: website.value,
        ...(widget !== null ? { turnstile_token: token } : {}),
      }),
    });
  } catch {
    res = null;
  }
  if (widget !== null) {
    token = "";
    window.turnstile.reset(widget);
  }
  submit.removeAttribute("aria-busy");
  submit.disabled = false;
  if (res && res.ok) {
    form.hidden = true;
    done.hidden = false;
    done.focus();
    return;
  }
  if (res && res.status === 429) {
    say("Too many tries from this network. Wait a little and try again.", true);
  } else if (res && res.status === 403) {
    say("Complete the check above, then try again.", true);
  } else if (res && res.status === 400) {
    say("That didn't look right. Check your email and note, then try again.", true);
  } else {
    say("We couldn't reach the server. Try again in a moment.", true);
  }
});

for (const input of [email, useCase, consent]) {
  input.addEventListener("input", () => {
    if (input.getAttribute("aria-invalid") === "true") validate();
  });
}

// Restart the lane animation. The settled state is in the stylesheet, so
// visitors who prefer reduced motion never see this control.
const floor = document.getElementById("floor");
const replay = document.getElementById("replay");
if (floor && replay) {
  replay.addEventListener("click", () => {
    floor.classList.remove("is-playing");
    void floor.offsetWidth;
    floor.classList.add("is-playing");
  });
}
