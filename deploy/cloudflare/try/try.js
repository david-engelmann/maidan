// The try page: start a sandbox, show its agent token once, set up a client,
// and sign the console in as the sandbox's admin.
//
// Secrets stay in this page's memory. The agent token is shown in the page;
// the admin token is never shown and never put in a URL: it is exchanged for
// an HttpOnly session cookie by a POST (`/auth/session/from-token`, the same
// call the console's own token sign-in makes), and then dropped.

import { claudeCommand, codexConfig, cursorLink, vscodeLink } from "/clients.js";

const $ = (id) => document.getElementById(id);

let adminToken = "";
let turnstileToken = "";
let turnstileWidget = null;

function say(el, text, error = false) {
  el.textContent = text;
  el.classList.toggle("error", error);
}

async function detail(res) {
  try {
    const body = await res.json();
    return typeof body?.detail === "string" ? body.detail : "";
  } catch {
    return "";
  }
}

function refusal(status, why) {
  if (status === 429) return "You have started the most sandboxes one address may this hour. Try again later.";
  if (status === 503) return "The demo is full until its daily reset. Try again later.";
  if (status === 404) return "This demo is not taking new sandboxes right now.";
  return why || `The sandbox could not be started (${status}). Try again.`;
}

async function loadTurnstile() {
  let siteKey = null;
  try {
    const res = await fetch("/try-config.json", { cache: "no-store" });
    siteKey = (await res.json())?.turnstile_site_key ?? null;
  } catch {
    siteKey = null;
  }
  if (!siteKey) return;
  const holder = $("turnstile");
  holder.hidden = false;
  await new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
    script.async = true;
    script.onload = resolve;
    script.onerror = reject;
    document.head.appendChild(script);
  });
  turnstileWidget = window.turnstile.render(holder, {
    sitekey: siteKey,
    action: "sandbox",
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
  $("agent-token").textContent = token;
  $("claude-cmd").textContent = claudeCommand(url, token);
  $("codex-config").textContent = codexConfig(url, token);
  $("cursor-link").href = cursorLink(url, token);
  $("vscode-link").href = vscodeLink(url, token);
  adminToken = sandbox.admin.token;
}

async function start() {
  const button = $("start");
  const status = $("start-status");
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
    say(status, refusal(res.status, await detail(res)), true);
    return;
  }
  fill(await res.json());
  say(status, "");
  button.hidden = true;
  $("turnstile").hidden = true;
  $("ready").hidden = false;
  $("ready-title").focus();
}

async function openConsole() {
  const button = $("console");
  const status = $("console-status");
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
      window.location.assign("/ui/");
      return;
    }
    say(status, refusal(res.status, await detail(res)), true);
  } catch {
    say(status, "Could not reach the demo. Try again.", true);
  }
  button.disabled = false;
}

async function copy(button) {
  const text = $(button.dataset.copy)?.textContent ?? "";
  let ok = false;
  try {
    await navigator.clipboard.writeText(text);
    ok = true;
  } catch {
    ok = false;
  }
  button.textContent = ok ? "Copied" : "Select and copy";
  button.classList.toggle("done", ok);
  setTimeout(() => {
    button.textContent = "Copy";
    button.classList.remove("done");
  }, ok ? 1400 : 4000);
}

$("start").addEventListener("click", () => {
  void start();
});
$("console").addEventListener("click", () => {
  void openConsole();
});
for (const button of document.querySelectorAll("button.copy")) {
  button.addEventListener("click", () => {
    void copy(button);
  });
}
loadTurnstile().catch(() => {
  say($("start-status"), "The human check did not load. Reload the page to try again.", true);
});
