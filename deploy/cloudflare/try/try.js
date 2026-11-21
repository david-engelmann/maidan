// The try page: start a sandbox, show its agent token once, set up a client
// or watch the page's own demo agent take the task, and sign the console in
// as the sandbox's admin.
//
// Secrets stay in this page's memory. The agent token is shown in the page,
// and the demo agent sends it only in a header to this origin; the admin
// token is never shown and never put in a URL: it is exchanged for an
// HttpOnly session cookie by a POST (`/auth/session/from-token`, the same
// call the console's own token sign-in makes), and then dropped.

import { claudeCommand, codexConfig, cursorLink, vscodeLink } from "./clients.js";
import { STEPS, createDemoAgent } from "./demo-agent.js";

// Keep in step with UNAVAILABLE_RETRY_SECS in src/edge.js. The Worker sends
// that delay on the 503 it writes while the demo container is starting.
export const START_RETRY_SECS = 20;

// The demo agent's pause between calls: long enough to read each step as it
// lands, short enough that the whole run takes a few seconds.
export const AGENT_PAUSE_MS = 700;

// How long the page waits for advice on the hand-off before it goes on
// without it. The server gives the model a second and a half.
export const ADVICE_WAIT_MS = 4000;

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

export function createTryPage(
  doc,
  fetch = globalThis.fetch,
  assign = (href) => window.location.assign(href),
  { agentPauseMs = AGENT_PAUSE_MS } = {},
) {
  let adminToken = "";
  let agent = null;
  const agentSteps = new Map();
  let turnstileToken = "";
  // Who the demo can ask for advice (`/try-config.json`); one or none shows no switch.
  let adviceProviders = [];
  let turnstileWidget = null;
  let turnstileFailed = false;
  const copyTimers = new WeakMap();

  async function loadTurnstile() {
    let siteKey = null;
    try {
      const res = await fetch("/try-config.json", { cache: "no-store" });
      const config = await res.json();
      siteKey = config?.turnstile_site_key ?? null;
      // Said before a sandbox exists: where this demo offers advice, a task
      // sent to review is read by a decision model outside it.
      if (config?.advice === true) {
        for (const note of doc.querySelectorAll?.(".advice-notice") ?? []) note.hidden = false;
      }
      // The decision models this demo can ask, for the switch on the advice.
      if (Array.isArray(config?.providers)) {
        adviceProviders = config.providers.filter((p) => typeof p?.id === "string" && typeof p?.name === "string");
      }
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
    agent = createDemoAgent({
      fetch,
      token,
      channelId: sandbox.channel_id,
      taskId: sandbox.task_id,
      retrySecs: START_RETRY_SECS,
      pauseMs: agentPauseMs,
    });
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

  // The ready section's console button, or the one the demo agent's card
  // shows once the task is in review: both sign in the same way.
  async function openConsole(button = $(doc, "console"), status = $(doc, "console-status")) {
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

  // One list item per step, made when the step starts, so the log grows as
  // the agent works. The log is not a live region: the status line under the
  // button names each step as it starts, so a screen reader hears it once.
  function showStep(index, state, detail) {
    if (state === "running") {
      say($(doc, "agent-status"), `Step ${index + 1} of ${STEPS.length}: ${STEPS[index].label}…`);
    }
    let item = agentSteps.get(index);
    if (!item) {
      const li = doc.createElement("li");
      li.className = "step";
      const head = doc.createElement("span");
      head.className = "step-head";
      const label = doc.createElement("span");
      label.className = "step-label";
      label.textContent = STEPS[index].label;
      const tool = doc.createElement("code");
      tool.textContent = STEPS[index].tool;
      const mark = doc.createElement("span");
      mark.className = "step-state";
      const note = doc.createElement("span");
      note.className = "step-detail";
      head.appendChild(label);
      head.appendChild(tool);
      head.appendChild(mark);
      li.appendChild(head);
      li.appendChild(note);
      const log = $(doc, "agent-log");
      log.appendChild(li);
      log.hidden = false;
      item = { li, mark, note };
      agentSteps.set(index, item);
    }
    item.li.dataset.state = state;
    item.mark.textContent = state === "done" ? "Done" : state === "failed" ? "Failed" : "Working…";
    item.note.textContent = detail;
  }

  // What a decision model read in the hand-off the agent just made, when
  // this demo offers advice. Without it, or when the model is slow or does
  // not answer, nothing is shown: the review is the same either way.
  // `provider` asks one model by name, from the switch.
  async function showAdvice(provider = null) {
    const box = $(doc, "agent-advice");
    if (!box) return;
    let timer;
    const late = new Promise((resolve) => {
      timer = setTimeout(() => resolve(null), ADVICE_WAIT_MS);
    });
    const advice = await Promise.race([agent.advice(provider), late]);
    clearTimeout(timer);
    const heard = $(doc, "advice-status");
    if (!advice) {
      // The first ask shows nothing. A switch that got no answer says so and
      // leaves the advice already on the page.
      if (provider && heard) say(heard, "That model did not answer. The advice shown is unchanged.");
      return;
    }
    box.dataset.land = advice.land;
    $(doc, "advice-model").textContent = advice.model;
    $(doc, "advice-title").textContent = advice.title;
    $(doc, "advice-why").textContent = advice.why;
    // What was flagged shows at once; all ten readings wait behind one press.
    const all = $(doc, "advice-rows");
    const flagged = $(doc, "advice-flagged");
    all.replaceChildren?.();
    flagged.replaceChildren?.();
    const line = (row) => {
      const li = doc.createElement("li");
      li.className = "advice-row";
      li.dataset.mark = row.mark;
      const mark = doc.createElement("span");
      mark.className = "advice-mark";
      mark.ariaHidden = "true";
      // The mark is a shape; a reader that cannot see it hears the word.
      const word = doc.createElement("span");
      word.className = "sr-only";
      word.textContent = row.mark === "concern" ? "Flagged: " : row.mark === "unsure" ? "Unsure: " : "";
      const reading = doc.createElement("span");
      reading.className = "advice-reading";
      reading.textContent = row.reading;
      const sure = doc.createElement("span");
      sure.className = "advice-sure";
      sure.textContent = row.sure;
      li.appendChild(mark);
      li.appendChild(word);
      li.appendChild(reading);
      li.appendChild(sure);
      return li;
    };
    for (const row of advice.rows) {
      all.appendChild(line(row));
      if (row.mark !== "clear") flagged.appendChild(line(row));
    }
    flagged.hidden = !advice.rows.some((row) => row.mark !== "clear");
    showProviders(advice.provider);
    box.hidden = false;
    // Said once to a screen reader, where the block's arrival would go
    // unheard; a sighted visitor reads it in the block itself.
    if (heard) say(heard, `A decision model read the hand-off: ${advice.title.toLowerCase()}.`);
  }

  // One button per decision model this demo can ask, the one that answered
  // pressed. Pressing another asks it about the same hand-off.
  function showProviders(answered) {
    const group = $(doc, "advice-providers");
    if (!group) return;
    if (adviceProviders.length < 2) {
      group.hidden = true;
      return;
    }
    group.replaceChildren?.();
    for (const p of adviceProviders) {
      const button = doc.createElement("button");
      button.type = "button";
      button.className = "advice-provider";
      button.textContent = p.name;
      button.ariaPressed = String(p.id === answered);
      button.addEventListener("click", () => {
        if (p.id === answered) return;
        for (const other of group.children) other.disabled = true;
        void showAdvice(p.id).finally(() => {
          for (const other of group.children) other.disabled = false;
        });
      });
      group.appendChild(button);
    }
    group.hidden = false;
  }

  async function watchAgent() {
    const button = $(doc, "agent-run");
    const status = $(doc, "agent-status");
    if (!agent) {
      say(status, "Start a sandbox first.", true);
      return;
    }
    button.disabled = true;
    const outcome = await agent.run(showStep);
    if (outcome.ok) {
      say(status, "The agent sent Say hello to review.");
      button.hidden = true;
      // Asked before the next step is shown, so the advice is in place when
      // it appears and nothing moves under the person's pointer.
      await showAdvice();
      $(doc, "agent-done").hidden = false;
      // The button that had focus is gone; the next thing to do is here.
      $(doc, "agent-console").focus();
      return;
    }
    say(status, `The agent stopped at step ${outcome.step + 1} of ${STEPS.length}. ${outcome.message}`, true);
    button.textContent = "Try again";
    button.disabled = false;
    // Disabling the focused button dropped focus to the page; put it back
    // where the next keypress tries again.
    button.focus();
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
    $(doc, "agent-run").addEventListener("click", () => {
      void watchAgent();
    });
    $(doc, "agent-console").addEventListener("click", () => {
      void openConsole($(doc, "agent-console"), $(doc, "agent-console-status"));
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

  // `loadConfig` is what `mount` does first: the human check, and what this demo offers.
  return { start, openConsole, watchAgent, mount, loadConfig: loadTurnstile };
}

if (typeof document !== "undefined") {
  createTryPage(document).mount();
}
