// The try page: start a sandbox, show its agent token once, set up a client,
// and sign the console in as the sandbox's admin.
//
// Secrets stay in this page's memory. The agent token is shown in the page;
// the admin token is never shown and never put in a URL: it is exchanged for
// an HttpOnly session cookie by a POST (`/auth/session/from-token`, the same
// call the console's own token sign-in makes), and then dropped.

import { claudeCommand, codexConfig, cursorLink, vscodeLink } from "./clients.js";

// Keep in step with UNAVAILABLE_RETRY_SECS in src/edge.js. The Worker sends
// that delay on the 503 it writes while the demo container is starting.
export const START_RETRY_SECS = 20;

const OVERLOADED = "https://maidan.world/problems/overloaded";
// POST /sandbox when the daily cap is hit (crates/maidan-server sandbox.rs).
// The Worker's container-starting 503 is the same status and problem type,
// and it carries Retry-After; the cap does not.
const DAILY_CAP = /full until its daily reset/i;

const $ = (doc, id) => doc.getElementById(id);

function say(el, text, error = false) {
  el.textContent = text;
  el.classList.toggle("error", error);
}

async function readProblem(res) {
  let body = null;
  try {
    body = await res.json();
  } catch {
    body = null;
  }
  return {
    why: typeof body?.detail === "string" ? body.detail : "",
    type: typeof body?.type === "string" ? body.type : "",
    retryAfter: res.headers.get("retry-after"),
  };
}

// `onSandbox` is POST /sandbox. The daily cap exists only there. A 503 from
// the console sign-in, or one that names a retry, is the container waking up
// or being replaced.
export function refusal(status, why, { type = "", retryAfter = null, onSandbox = false } = {}) {
  if (status === 429) return "You have started the most sandboxes one address may this hour. Try again later.";
  if (status === 503) {
    const retry = retryAfter !== null && retryAfter !== undefined && String(retryAfter).trim() !== "";
    const dailyCap =
      onSandbox &&
      DAILY_CAP.test(why) &&
      !retry &&
      (type === "" || type === OVERLOADED);
    if (dailyCap) return "The demo is full until its daily reset. Try again later.";
    return `The demo is starting or being updated. Try again in about ${START_RETRY_SECS} seconds.`;
  }
  if (status === 404) return "This demo is not taking new sandboxes right now.";
  if (status >= 500) {
    if (/human check/i.test(why)) return sentence(why);
    return "The sandbox could not be started. Try again.";
  }
  return why || `The sandbox could not be started (${status}). Try again.`;
}

function sentence(why) {
  const text = why.trim();
  const lead = text.charAt(0).toUpperCase() + text.slice(1);
  return /[.!?]$/.test(lead) ? lead : `${lead}.`;
}

export function createTryPage(doc, fetch = globalThis.fetch, assign = (href) => window.location.assign(href)) {
  let adminToken = "";
  let turnstileToken = "";
  let turnstileWidget = null;
  let turnstileFailed = false;
  const copyTimers = new WeakMap();

  async function loadTurnstile() {
    let siteKey = null;
    try {
      const res = await fetch("/try-config.json", { cache: "no-store" });
      siteKey = (await res.json())?.turnstile_site_key ?? null;
    } catch {
      siteKey = null;
    }
    const holder = $(doc, "turnstile");
    if (!siteKey) {
      if (holder) holder.hidden = true;
      return;
    }
    if (holder) holder.hidden = true;
    const script = doc.createElement("script");
    script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
    script.async = true;
    try {
      await new Promise((resolve, reject) => {
        script.onload = resolve;
        script.onerror = reject;
        doc.head.appendChild(script);
      });
    } catch {
      script.remove?.();
      if (holder) holder.hidden = true;
      turnstileFailed = true;
      throw new Error("turnstile");
    }
    if (!window.turnstile?.render) {
      script.remove?.();
      if (holder) holder.hidden = true;
      turnstileFailed = true;
      return;
    }
    turnstileFailed = false;
    holder.hidden = false;
    turnstileWidget = window.turnstile.render(holder, {
      sitekey: siteKey,
      action: "sandbox",
      // The page's theme, which /theme.js set from the stored choice.
      theme: doc.documentElement?.getAttribute?.("data-theme") === "light" ? "light" : "dark",
      callback: (token) => {
        turnstileToken = token;
      },
      "expired-callback": () => {
        turnstileToken = "";
      },
    });
  }

  function fill(sandbox) {
    const token = sandbox.agent.token;
    const url = sandbox.mcp_url;
    $(doc, "agent-token").textContent = token;
    $(doc, "claude-cmd").textContent = claudeCommand(url, token);
    $(doc, "codex-config").textContent = codexConfig(url, token);
    $(doc, "cursor-link").href = cursorLink(url, token);
    $(doc, "vscode-link").href = vscodeLink(url, token);
    adminToken = sandbox.admin.token;
  }

  async function start() {
    const button = $(doc, "start");
    const status = $(doc, "start-status");
    if (turnstileFailed) {
      try {
        await loadTurnstile();
      } catch {
        say(status, "The human check did not load. Reload the page to try again.", true);
        return;
      }
      if (turnstileFailed) {
        say(status, "The human check did not load. Reload the page to try again.", true);
        return;
      }
    }
    if (turnstileWidget !== null && !turnstileToken) {
      say(status, "Complete the human check first.", true);
      return;
    }
    button.disabled = true;
    say(status, "Starting your sandbox…");
    let res;
    try {
      res = await fetch("/sandbox", {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify(turnstileToken ? { turnstile_token: turnstileToken } : {}),
        cache: "no-store",
        credentials: "same-origin",
      });
    } catch {
      button.disabled = false;
      say(status, "Could not reach the demo. Check your connection and try again.", true);
      return;
    }
    if (res.status !== 201) {
      button.disabled = false;
      if (turnstileWidget !== null) {
        turnstileToken = "";
        window.turnstile.reset(turnstileWidget);
      }
      const problem = await readProblem(res);
      say(status, refusal(res.status, problem.why, { ...problem, onSandbox: true }), true);
      return;
    }
    fill(await res.json());
    say(status, "");
    button.hidden = true;
    $(doc, "turnstile").hidden = true;
    $(doc, "ready").hidden = false;
    const heroNotice = $(doc, "hero-notice");
    if (heroNotice) heroNotice.hidden = true;
    $(doc, "ready-title").focus();
  }

  async function openConsole() {
    const button = $(doc, "console");
    const status = $(doc, "console-status");
    if (!adminToken) {
      say(status, "Start a sandbox first.", true);
      return;
    }
    button.disabled = true;
    say(status, "Signing you in…");
    try {
      const res = await fetch("/auth/session/from-token", {
        method: "POST",
        headers: { accept: "application/json", authorization: `Bearer ${adminToken}` },
        cache: "no-store",
        credentials: "same-origin",
      });
      if (res.ok) {
        assign("/ui/");
        return;
      }
      const problem = await readProblem(res);
      say(status, refusal(res.status, problem.why, { ...problem, onSandbox: false }), true);
    } catch {
      say(status, "Could not reach the demo. Try again.", true);
    }
    button.disabled = false;
  }

  async function copy(button) {
    const text = $(doc, button.dataset.copy)?.textContent ?? "";
    let ok = false;
    try {
      await navigator.clipboard.writeText(text);
      ok = true;
    } catch {
      ok = false;
    }
    button.textContent = ok ? "Copied" : "Select and copy";
    button.classList.toggle("done", ok);
    const previous = copyTimers.get(button);
    if (previous) clearTimeout(previous);
    const timer = setTimeout(() => {
      button.textContent = "Copy";
      button.classList.remove("done");
      copyTimers.delete(button);
    }, ok ? 1400 : 4000);
    copyTimers.set(button, timer);
  }

  function mount() {
    $(doc, "start").addEventListener("click", () => {
      void start();
    });
    $(doc, "console").addEventListener("click", () => {
      void openConsole();
    });
    for (const button of doc.querySelectorAll("button.copy")) {
      button.addEventListener("click", () => {
        void copy(button);
      });
    }
    loadTurnstile().catch(() => {
      say($(doc, "start-status"), "The human check did not load. Reload the page to try again.", true);
    });
  }

  return { start, openConsole, mount };
}

if (typeof document !== "undefined") {
  createTryPage(document).mount();
}
