// @ts-check
import { api, apiReadPath, base, headers, token, wid, writeApi } from "./api.js";
import { escapeHtml } from "./artifacts.js";
import { responseError, showError, unreachable } from "./feedback.js";
import { ago, leaseLeft, loadMembers, memberName } from "./people.js";

// The swarm panel: what the workers in this workspace hold, what waits and
// why, which slots are full, and which repositories are paused. It reads
// only the four footprint lists the server already serves; like peers and
// token minting it stays on the bearer tree, so a signed-in session without
// a pasted token is told so in a sentence instead of reading raw errors.

const REASON_LABELS = {
  overlap: "A live claim overlaps its footprint",
  repo_paused: "Its repository is paused",
  slot_full: "A slot it needs is at capacity",
};

/** @param {string} id */
function box(id) {
  return document.getElementById(id);
}

/** @param {string} id @param {string} html */
function setBox(id, html) {
  const el = box(id);
  if (el) el.innerHTML = html;
}

/** @param {unknown} e */
function sectionError(id, e) {
  setBox(id, `<p class="muted">Could not load this list. ${escapeHtml(e instanceof Error ? e.message : String(e))}</p>`);
}

/**
 * One GET against the bearer tree, parsed as JSON. The URL is built from
 * base() directly, not through the read/write path helpers: those promise
 * a session-proxy twin these lists do not have.
 * @param {string} suffix
 * @returns {Promise<any>}
 */
async function fetchList(suffix) {
  const res = await api(`${base()}${suffix}`, { headers: headers(), credentials: "include" });
  if (!res.ok) throw new Error(await responseError(res, "The server refused the list"));
  return res.json();
}

const METER_LABELS = {
  requests: "Requests",
  mcp_calls: "MCP tool calls",
  embedding_calls: "Embeddings",
  deliveries: "Deliveries sent",
  wasi_ms: "WASI run time (ms)",
  stored_bytes: "Stored bytes",
};

/** @param {any} usage */
function renderUsage(usage) {
  const rows = (usage.meters || [])
    .map((m) => {
      const label = escapeHtml(METER_LABELS[m.meter] || m.meter);
      const of = m.allowance == null ? "" : ` <span class="muted">of ${escapeHtml(String(m.allowance))} free</span>`;
      const over = m.over ? ` <strong>Past the free allowance.</strong>` : "";
      return `<li class="swarm-row">${label}: ${escapeHtml(String(m.quantity))}${of}${over}</li>`;
    })
    .join("");
  const note = `<p class="muted">Nothing is cut off or billed; past an allowance, the workspace's admins are told.</p>`;
  setBox("swarm-usage", `<ul class="swarm-list" data-month="${escapeHtml(usage.month)}">${rows}</ul>${note}`);
}

// Usage is served to a signed-in session too, so it loads whether or not a
// token is pasted, unlike the footprint lists below it.
/** @param {string} id @param {() => boolean} current */
async function loadUsage(id, current) {
  try {
    const res = await api(apiReadPath(`/workspaces/${id}/usage-meters`), {
      headers: headers(),
      credentials: "include",
    });
    if (!res.ok) throw new Error(await responseError(res, "The server refused the usage"));
    const usage = await res.json();
    if (current()) renderUsage(usage);
  } catch (e) {
    if (current()) sectionError("swarm-usage", e);
  }
}

/** @param {any[]} claims */
function renderClaims(claims) {
  if (!claims.length) {
    setBox("swarm-claims", `<p class="muted">No live claims. Nothing is being worked on right now.</p>`);
    return;
  }
  const rows = claims
    .map((c) => {
      const items = (c.items || [])
        .map((i) => `<li><code>${escapeHtml(i.kind)}</code> ${escapeHtml(i.value)}</li>`)
        .join("");
      const lease = c.lease_expires_at ? escapeHtml(leaseLeft(c.lease_expires_at)) : "no lease";
      return `<li class="swarm-row">
        <div><strong>${escapeHtml(memberName(c.assignee_id))}</strong> holds <code>${escapeHtml(c.repo)}</code>
        <span class="muted">(${escapeHtml(c.mode)}, ${lease})</span></div>
        ${items ? `<ul class="swarm-items">${items}</ul>` : ""}
      </li>`;
    })
    .join("");
  setBox("swarm-claims", `<ul class="swarm-list">${rows}</ul>`);
}

/** @param {any[]} deferred */
function renderDeferred(deferred) {
  if (!deferred.length) {
    setBox("swarm-deferred", `<p class="muted">No thread is being held back by its footprint. Threads waiting for other reasons are not listed here.</p>`);
    return;
  }
  const rows = deferred
    .map((d) => {
      const title = d.title ? escapeHtml(d.title) : `thread ${escapeHtml(String(d.thread_id).slice(0, 8))}`;
      const why = (d.reasons || [])
        .map((r) => escapeHtml(REASON_LABELS[r] || r))
        .join("; ");
      return `<li class="swarm-row"><div><strong>${title}</strong></div><div class="muted">${why}</div></li>`;
    })
    .join("");
  setBox("swarm-deferred", `<ul class="swarm-list">${rows}</ul>`);
}

/** @param {any[]} slots */
function renderSlots(slots) {
  if (!slots.length) {
    setBox("swarm-slots", `<p class="muted">No slots in use.</p>`);
    return;
  }
  const rows = slots
    .map((s) => {
      const cap = s.capacity == null ? "no limit set" : `of ${s.capacity} allowed`;
      const full = s.capacity != null && s.in_use >= s.capacity ? ` <strong>Full.</strong>` : "";
      return `<li class="swarm-row"><code>${escapeHtml(s.slot)}</code> <span class="muted">${s.in_use} in use ${escapeHtml(cap)}</span>${full}</li>`;
    })
    .join("");
  setBox("swarm-slots", `<ul class="swarm-list">${rows}</ul>`);
}

/** @param {any[]} pauses @param {string} workspace */
function renderPauses(pauses, workspace) {
  if (!pauses.length) {
    setBox("swarm-pauses", `<p class="muted">No repositories are paused.</p>`);
    return;
  }
  const rows = pauses
    .map((p) => {
      const reason = p.reason ? ` <span class="muted">${escapeHtml(p.reason)}</span>` : "";
      return `<li class="swarm-row">
        <div><code>${escapeHtml(p.repo)}</code> <span class="muted">paused by ${escapeHtml(memberName(p.paused_by))}, ${escapeHtml(ago(p.paused_at))}</span>${reason}</div>
        <button type="button" class="ghost" data-resume="${escapeHtml(p.repo)}" data-resume-workspace="${escapeHtml(workspace)}">Resume</button>
      </li>`;
    })
    .join("");
  setBox("swarm-pauses", `<ul class="swarm-list">${rows}</ul>`);
  box("swarm-pauses")
    ?.querySelectorAll("button[data-resume]")
    .forEach((btn) => {
      const el = /** @type {HTMLElement} */ (btn);
      btn.addEventListener("click", () =>
        resumeRepo(el.dataset.resume || "", el.dataset.resumeWorkspace || "", /** @type {HTMLButtonElement} */ (btn)),
      );
    });
}

// Loads overlap (tab click, refresh timer, a pause or resume finishing). A
// load may only paint while it is still the newest one and the workspace
// it read for is still the selected one; otherwise its rows would be a
// stale snapshot, or another workspace's.
let swarmLoadGen = 0;
let usageLoadGen = 0;

async function loadSwarm() {
  if (!wid()) {
    ["swarm-usage", "swarm-claims", "swarm-deferred", "swarm-slots", "swarm-pauses"].forEach((id) =>
      setBox(id, `<p class="muted">Set a workspace first.</p>`),
    );
    return;
  }
  const usageGen = ++usageLoadGen;
  const usageFor = wid();
  loadUsage(usageFor, () => usageGen === usageLoadGen && wid() === usageFor);
  if (!token()) {
    const note = `<p class="muted">This panel reads the bearer API, like peers and token minting. Paste a token under Connect this browser to use it.</p>`;
    ["swarm-claims", "swarm-deferred", "swarm-slots", "swarm-pauses"].forEach((id) => setBox(id, note));
    return;
  }
  const gen = ++swarmLoadGen;
  const id = wid();
  const current = () => gen === swarmLoadGen && wid() === id;
  // Names first, so holders and pausers render as people, not raw ids.
  await loadMembers();
  /** @type {[string, string, (rows: any[]) => void][]} */
  const jobs = [
    ["swarm-claims", `/workspaces/${id}/claims`, renderClaims],
    ["swarm-deferred", `/workspaces/${id}/deferred`, renderDeferred],
    ["swarm-slots", `/workspaces/${id}/slots`, renderSlots],
    ["swarm-pauses", `/workspaces/${id}/repo-pauses`, (rows) => renderPauses(rows, id)],
  ];
  // Each list lands on its own: one failing list must not blank the rest.
  await Promise.all(
    jobs.map(async ([boxId, suffix, render]) => {
      try {
        const rows = await fetchList(/** @type {string} */ (suffix));
        if (current()) render(rows);
      } catch (e) {
        if (current()) sectionError(/** @type {string} */ (boxId), e);
      }
    }),
  );
}

/** @param {string} repo @param {string} workspace @param {HTMLButtonElement} button */
async function resumeRepo(repo, workspace, button) {
  if (!repo) return;
  // The row names the workspace it was loaded for. Resuming acts on the
  // workspace selected now, so a row left over from another workspace
  // must not fire: reload for the selected workspace instead.
  if (workspace !== wid()) {
    showError("The workspace changed since this list loaded. It has been reloaded for the workspace now selected.");
    await loadSwarm();
    return;
  }
  let res;
  try {
    res = await writeApi(button, `${base()}/workspaces/${wid()}/repo-pauses?repo=${encodeURIComponent(repo)}`, {
      method: "DELETE",
      headers: headers(),
      credentials: "include",
    });
  } catch (e) {
    showError(unreachable(e));
    return;
  }
  if (!res.ok) {
    showError(await responseError(res, "Could not resume that repository"));
    return;
  }
  await loadSwarm();
}

async function pauseRepo() {
  const input = /** @type {HTMLInputElement | null} */ (box("swarm-pause-repo"));
  const reasonInput = /** @type {HTMLInputElement | null} */ (box("swarm-pause-reason"));
  const button = /** @type {HTMLButtonElement | null} */ (box("swarm-pause-submit"));
  const repo = (input?.value || "").trim();
  if (!repo) {
    showError("Name the repository to pause, as owner/name.");
    return;
  }
  let res;
  try {
    res = await writeApi(button, `${base()}/workspaces/${wid()}/repo-pauses`, {
      method: "PUT",
      headers: headers(true),
      credentials: "include",
      body: JSON.stringify({ repo, reason: (reasonInput?.value || "").trim() || undefined }),
    });
  } catch (e) {
    showError(unreachable(e));
    return;
  }
  if (!res.ok) {
    showError(await responseError(res, "Could not pause that repository"));
    return;
  }
  if (input) input.value = "";
  if (reasonInput) reasonInput.value = "";
  await loadSwarm();
}

function initSwarm() {
  box("swarm-refresh")?.addEventListener("click", () => loadSwarm());
  box("swarm-pause-submit")?.addEventListener("click", () => pauseRepo());
  // While the panel is open it stays current on the board's own cadence;
  // a closed panel costs no requests.
  setInterval(() => {
    if (box("panel-swarm")?.classList.contains("active")) loadSwarm();
  }, 30000);
}

export { initSwarm, loadSwarm };
