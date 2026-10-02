// @ts-check
import { api, apiReadPath, base, headers, persist, token, uiReadPath, wid } from "./api.js";
import { escapeHtml } from "./artifacts.js";
import { clearLoading, keyActivates, renderState, responseError, setLoading, unreachable } from "./feedback.js";
import { loadNeedsYou, needsYou, renderThreadActions, syncCollabPanel } from "./needs.js";
import { openConnect } from "./palette.js";
import { ago, authorId, avatarEl, leaseLeft, loadMembers, personEl } from "./people.js";
import { sessionMemberId } from "./session.js";
import { ACTOR_KEYS, BOARD_COLUMNS, LIVE_MS, SUBJECT_KEYS, channelKey, lastSeen, memberDirectory, reduceMotion, refusals } from "./state.js";
import { loadMessages } from "./thread.js";


      let selectedChannelId = null;

      let selectedThreadId = null;

      let threadsById = new Map();

      let lastGates = {};

      let boardSeen = new Map();

      let selectedChannelName = "";


      let threadLoadGen = 0;

      let boardShownFor = null;

      async function loadThreads(quiet = false) {
        // Only the newest load paints: a slow reply for channel A never paints
        // over B, and an older reply for the same channel never replaces a
        // newer one.
        const channelId = selectedChannelId;
        const gen = ++threadLoadGen;
        const stale = () => selectedChannelId !== channelId || gen !== threadLoadGen;
        if (!channelId) return;
        // A board already showing this channel keeps its cards while it
        // reloads (so they can glide); a newly picked one says it is loading.
        const replacing = boardShownFor !== channelId;
        if (replacing) boardState("loading");
        persist();
        const fail = (message) => {
          if (stale()) return;
          if (replacing) boardState("error", message);
          else boardNote(`Last refresh failed: ${message}`);
        };
        try {
          // The endpoint pages by thread id; keep asking until a short page, so
          // a channel with more threads than one page still gets every card.
          const threads = [];
          const pageSize = 500;
          let cursor = null;
          // No page cap: a large channel gets every card. A cursor that does
          // not advance would loop forever, so that stops the walk instead.
          for (;;) {
            const q = new URLSearchParams({ limit: String(pageSize) });
            if (cursor) q.set("cursor", cursor);
            const res = await api(
              uiReadPath(`/channels/${channelId}/threads?${q}`),
              { headers: headers(), credentials: "include" }
            );
            if (!res.ok) return fail(await responseError(res, `Could not load #${selectedChannelName || "this channel"}`));
            const batch = await res.json();
            if (stale()) return;
            threads.push(...batch);
            if (batch.length < pageSize) break;
            const next = batch[batch.length - 1].id;
            if (next === cursor) break;
            cursor = next;
          }
          const gates = await fetchPendingGatesByThread();
          if (stale()) return;
          renderBoard(threads, gates);
          boardShownFor = channelId;
          hydrateRefusals(threads);
          loadNeedsYou();
          if (!threads.length) {
            document.getElementById("board-summary").textContent =
              "No tasks in this channel yet. Add one above, or let an agent post one.";
          }
        } catch (e) {
          fail(unreachable(e));
        }
      }

      // Loading and error states for the board itself, so a picked channel never
      // shows the sign-in help while its tasks load or fail to.
      function boardState(kind, message) {
        boardShownFor = null;
        const board = document.getElementById("board");
        const name = selectedChannelName ? `#${selectedChannelName}` : "this channel";
        document.getElementById("board-title").textContent = selectedChannelName ? `# ${selectedChannelName}` : "Board";
        document.getElementById("board-summary").textContent = "";
        const box = document.createElement("div");
        box.className = `onboard board-${kind}`;
        box.id = "board-onboard";
        const h = document.createElement("h3");
        if (kind === "loading") {
          box.setAttribute("aria-busy", "true");
          h.textContent = `Loading ${name}…`;
          box.append(h);
          board.setAttribute("aria-busy", "true");
        } else {
          box.setAttribute("role", "alert");
          h.textContent = `Could not load ${name}`;
          const p = document.createElement("p");
          p.textContent = message;
          const row = document.createElement("div");
          row.className = "row";
          const retry = document.createElement("button");
          retry.type = "button";
          retry.className = "primary";
          retry.textContent = "Try again";
          retry.onclick = () => loadThreads();
          row.append(retry);
          box.append(h, p, row);
          board.removeAttribute("aria-busy");
        }
        board.replaceChildren(box);
      }

      function boardNote(text) {
        const summary = document.getElementById("board-summary");
        const note = document.createElement("span");
        note.className = "board-note";
        note.textContent = text;
        summary.querySelector(".board-note")?.remove();
        summary.appendChild(note);
      }


      // Pending approval gates for the current workspace, keyed by thread id, so
      // loadThreads can flag a gated thread. Best-effort: on any failure the
      // badges simply omit the gate state.
      async function fetchPendingGatesByThread() {
        const ws = wid();
        const map = {};
        if (!ws) return map;
        try {
          const res = await api(uiReadPath(`/workspaces/${ws}/approval-gates`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) return map;
          const views = await res.json();
          pendingGateViews = new Map();
          for (const v of views) {
            const g = v.gate;
            if (g) pendingGateViews.set(g.id, v);
            if (g && g.thread_id) map[g.thread_id] = { hasSchema: g.schema != null };
          }
        } catch (_e) {
          /* best-effort */
        }
        return map;
      }


      // Session chrome for a task thread — first match wins. It reads the
      // thread's real FSM state and claim, so nothing is "idle" by default:
      //  done           — thread terminal (closed/archived)
      //  needs-input    — a pending gate with a schema (structured elicitation)
      //  needs-approval — a pending gate without a schema (yes/no approval)
      //  in review      — FSM state in_review (a result is waiting on review)
      //  running        — claimed AND the working clock has started
      //  claimed        — claimed, working clock not started yet
      //  open           — unclaimed: free for the next agent to take
      // `column` places the thread on the board.
      function sessionChrome(th, gate) {
        if (th.state === "closed" || th.state === "archived")
          return { label: "done", cls: "chrome-done", hint: "thread is closed", column: "done" };
        if (gate)
          return gate.hasSchema
            ? { label: "needs-input", cls: "chrome-needs-input", hint: "waiting on structured input", column: "review" }
            : { label: "needs-approval", cls: "chrome-needs-approval", hint: "waiting on human approval", column: "review" };
        if (th.state === "in_review")
          return { label: "in review", cls: "chrome-in-review", hint: "result posted, waiting on review", column: "review" };
        if (th.assignee_id && th.work_started_at)
          return { label: "running", cls: "chrome-running", hint: "claimed and working", column: "working" };
        if (th.assignee_id)
          return { label: "claimed", cls: "chrome-claimed", hint: "claimed, not started yet", column: "working" };
        return { label: "open", cls: "chrome-open", hint: "unclaimed, free to take", column: "open" };
      }

      function paintRefusal() {
        const banner = document.getElementById("board-refusal");
        if (!banner) return;
        // Do not paint #board-refusal. A refusal that arrives before the card
        // exists used to open this strip. The strip stays hidden; the card draws it.
        banner.hidden = true;
        banner.replaceChildren();
        delete banner.dataset.threadId;
      }

      function rememberRefusal(threadId, actorId, text, at) {
        if (!threadId || !text) return;
        const stamp = at || new Date().toISOString();
        const prev = refusals.get(threadId);
        // A later notice replaces an earlier one. An older fetch must not
        // cover a refusal that already arrived live.
        if (prev && prev.at && String(prev.at) > String(stamp)) {
          paintRefusal();
          return;
        }
        refusals.set(threadId, { actorId, text, at: stamp });
        paintRefusal();
        const card = document.querySelector(`#board .card[data-id="${CSS.escape(threadId)}"]`);
        if (card && text) card.title = text;
        if (card && !card.querySelector(".card-refusal")) {
          const line = document.createElement("div");
          line.className = "card-refusal";
          line.textContent = "Close refused";
          card.appendChild(line);
        }
      }

      function noteRefusalFromFrame(v) {
        const meta = v && v.message && v.message.metadata;
        if (!meta || meta.notice !== "transition_refused") return;
        rememberRefusal(v.thread_id, v.message.author_id, v.message.body, (v.message && v.message.created_at) || (v.occurred_at));
      }

      // One notices read for the channel, not one history fetch per thread.
      // The board already did the one gates read in fetchPendingGatesByThread.
      // A notice is a message_posted event whose metadata.notice is
      // transition_refused. rememberRefusal keeps a newer live sentence when
      // this page is older. The strip above the lanes stays hidden.
      async function hydrateRefusals(threads) {
        if (!token() && !sessionMemberId) return;
        const pending = threads.filter((th) => th.state === "in_review");
        if (!pending.length || !wid() || !selectedChannelId) return;
        const wanted = new Set(pending.map((th) => th.id));
        try {
          const q = new URLSearchParams({
            types: "message_posted",
            channel_id: selectedChannelId,
            limit: "500",
          });
          const res = await api(
            apiReadPath(`/workspaces/${wid()}/events?${q}`),
            { headers: headers(), credentials: "include" }
          );
          if (res.ok) {
            const events = await res.json();
            for (const ev of events) {
              const payload = ev.payload || {};
              const message = payload.message;
              const threadId = (message && message.thread_id) || payload.thread_id || ev.thread_id;
              if (!wanted.has(threadId) || !message) continue;
              const meta = message.metadata;
              if (!meta || meta.notice !== "transition_refused") continue;
              rememberRefusal(
                threadId,
                message.author_id,
                message.body,
                message.created_at || payload.occurred_at
              );
            }
          }
        } catch (_e) {
          /* the live frame is the other way this arrives */
        }
        paintRefusal();
        pending.forEach((th) => {
          const rec = refusals.get(th.id);
          if (!rec) return;
          const card = document.querySelector(`#board .card[data-id="${CSS.escape(th.id)}"]`);
          if (!card) return;
          if (rec.text) card.title = rec.text;
          if (!card.querySelector(".card-refusal")) {
            const line = document.createElement("div");
            line.className = "card-refusal";
            line.textContent = "Close refused";
            card.appendChild(line);
          }
        });
      }


      function renderBoard(threads, gates) {
        threadsById = new Map(threads.map((th) => [th.id, th]));
        lastGates = gates;
        const board = document.getElementById("board");
        const summary = document.getElementById("board-summary");
        document.getElementById("board-title").textContent = selectedChannelName
          ? `# ${selectedChannelName}`
          : "Board";
        const before = boardRects();
        board.removeAttribute("aria-busy");
        board.replaceChildren();
        if (!threads.length && selectedChannelId) {
          board.appendChild(emptyChannelHelp());
          const firstPaint = boardSeen.size === 0;
          boardSeen = new Map();
          summary.replaceChildren();
          if (!firstPaint) glideCards(before);
          renderTeam(threads);
          if (selectedThreadId && threadsById.has(selectedThreadId)) renderThreadHeader();
          return;
        }
        const cols = {};
        BOARD_COLUMNS.forEach((c) => {
          const col = document.createElement("section");
          col.className = "board-col";
          col.dataset.column = c.key;
          const h = document.createElement("h3");
          const label = document.createElement("span");
          label.textContent = c.title;
          const count = document.createElement("span");
          h.append(label, " ", count);
          col.appendChild(h);
          board.appendChild(col);
          cols[c.key] = { el: col, count, n: 0 };
        });
        const nextSeen = new Map();
        const agents = new Set();
        threads.forEach((th) => {
          const chrome = sessionChrome(th, gates[th.id]);
          const title = th.title || th.id.slice(0, 8);
          const card = document.createElement("article");
          card.className = "card";
          card.dataset.id = th.id;
          card.dataset.chrome = chrome.label;
          card.tabIndex = 0;
          card.setAttribute("role", "button");
          card.setAttribute("aria-label", `${title}: ${chrome.label}`);
          if (th.id === selectedThreadId) card.classList.add("selected");
          const sig = `${chrome.label}|${th.assignee_id || ""}`;
          if (boardSeen.size && boardSeen.get(th.id) !== sig) card.classList.add("fresh");
          nextSeen.set(th.id, sig);
          const t = document.createElement("div");
          t.className = "card-title";
          t.textContent = title;
          const foot = document.createElement("div");
          foot.className = "card-foot";
          const stateWord = document.createElement("span");
          stateWord.className = "card-state";
          stateWord.textContent = chrome.label;
          foot.appendChild(stateWord);
          if (th.assignee_id) {
            foot.appendChild(personEl(th.assignee_id));
            if (chrome.column === "working") agents.add(th.assignee_id);
          } else {
            const u = document.createElement("span");
            u.className = "unclaimed";
            u.textContent = chrome.column === "done" ? "" : "unclaimed";
            foot.appendChild(u);
          }
          const when = document.createElement("span");
          when.className = "when";
          when.textContent =
            chrome.column === "working" && th.assignment_expires_at
              ? leaseLeft(th.assignment_expires_at)
              : ago(th.updated_at);
          foot.appendChild(when);
          card.append(t, foot);
          const refused = refusals.get(th.id);
          if (refused) {
            const line = document.createElement("div");
            line.className = "card-refusal";
            line.textContent = "Close refused";
            card.appendChild(line);
            if (refused.text) card.title = refused.text;
          }
          card.onclick = () => selectThread(th.id, title);
          card.onkeydown = (e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              selectThread(th.id, title);
            }
          };
          const col = cols[chrome.column];
          col.el.appendChild(card);
          col.n += 1;
        });
        const firstPaint = boardSeen.size === 0;
        boardSeen = nextSeen;
        BOARD_COLUMNS.forEach((c) => {
          const col = cols[c.key];
          col.count.textContent = String(col.n);
          if (!col.n) {
            const empty = document.createElement("div");
            empty.className = "board-empty";
            empty.textContent = "—";
            col.el.appendChild(empty);
          }
        });
        summary.replaceChildren();
        const facts = [
          [threads.length, threads.length === 1 ? "task" : "tasks"],
          [agents.size, agents.size === 1 ? "agent working" : "agents working"],
          [cols.review.n, "waiting on review"],
          [cols.done.n, "done"],
        ];
        facts.forEach(([n, label]) => {
          const span = document.createElement("span");
          const b = document.createElement("b");
          b.textContent = String(n);
          span.append(b, ` ${label}`);
          summary.appendChild(span);
        });
        if (!firstPaint) glideCards(before);
        renderTeam(threads);
        if (selectedThreadId && threadsById.has(selectedThreadId)) renderThreadHeader();
      }

      let boardChannel = null;

      function boardRects() {
        const rects = new Map();
        if (boardChannel !== selectedChannelId) {
          boardChannel = selectedChannelId;
          return rects;
        }
        document.querySelectorAll("#board .card").forEach((c) => rects.set(c.dataset.id, c.getBoundingClientRect()));
        return rects;
      }

      function glideCards(before) {
        if (reduceMotion.matches || !before.size) return;
        document.querySelectorAll("#board .card").forEach((card) => {
          const was = before.get(card.dataset.id);
          if (!was) {
            card.animate(
              [{ opacity: 0, transform: "scale(0.96)" }, { opacity: 1, transform: "none" }],
              { duration: 320, easing: "ease-out" }
            );
            return;
          }
          const now = card.getBoundingClientRect();
          const dx = was.left - now.left;
          const dy = was.top - now.top;
          if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
          card.animate(
            [
              { transform: `translate(${dx}px, ${dy}px)`, zIndex: 2 },
              { transform: "none", zIndex: 2 },
            ],
            { duration: 520, easing: "cubic-bezier(0.2, 0.8, 0.2, 1)" }
          );
        });
      }

      function markSeen(id) {
        if (id && memberDirectory.has(id)) lastSeen.set(id, Date.now());
      }

      // Redraw the strip after socket activity (coalesced), and every 30 s so
      // a dot goes grey once its two minutes are up.
      let teamTimer = null;

      function refreshTeamSoon() {
        if (teamTimer) return;
        teamTimer = setTimeout(() => {
          teamTimer = null;
          renderTeam([...threadsById.values()]);
        }, 150);
      }

      function markSeenFromFrame(v) {
        if (!v) return;
        const skip = SUBJECT_KEYS[v.kind] || [];
        for (const k of ACTOR_KEYS) if (!skip.includes(k) && typeof v[k] === "string") markSeen(v[k]);
        if (v.message && typeof v.message.author_id === "string") markSeen(v.message.author_id);
        if (v.attribution && typeof v.attribution.actor_id === "string") markSeen(v.attribution.actor_id);
      }

      function renderTeam(threads) {
        const box = document.getElementById("team");
        box.replaceChildren();
        if (!memberDirectory.size || !threads.length) return;
        const holding = new Map();
        const updatedAt = (row) => {
          const n = Date.parse((row && row.updated_at) || "");
          return Number.isNaN(n) ? -Infinity : n;
        };
        threads.forEach((th) => {
          if (th.assignee_id && th.state !== "closed" && th.state !== "archived") {
            const chrome = sessionChrome(th, lastGates[th.id]);
            const cur = holding.get(th.assignee_id);
            // Two open tasks: the running one, else the latest updated_at.
            const nextRun = chrome.label === "running";
            const curRun = !!(cur && cur.chrome.label === "running");
            const later = cur && updatedAt(th) > updatedAt(cur.th);
            if (!cur || (nextRun && !curRun) || (nextRun === curRun && later)) {
              holding.set(th.assignee_id, { th, chrome });
            }
          }
        });
        const waiting = needsYou.length;
        const people = [...memberDirectory.entries()]
          .filter(([, m]) => m.kind === "agent" || m.kind === "human")
          .sort((a, b) => (a[1].kind === b[1].kind ? 0 : a[1].kind === "agent" ? -1 : 1));
        const now = Date.now();
        people.forEach(([id, m]) => {
          const held = holding.get(id);
          const state = held ? (held.chrome.label === "running" ? "running" : "holding") : "idle";
          const live = state === "running" || now - (lastSeen.get(id) || 0) < LIVE_MS;
          // Only people holding work. Idle is not a chip, except the viewer
          // while something is waiting on them.
          if (state === "idle" && !(id === authorId() && waiting)) return;
          const chip = document.createElement("span");
          chip.className = "mate";
          chip.dataset.memberId = id;
          chip.dataset.state = state;
          chip.dataset.live = String(live || id === authorId());
          chip.title = id;
          const av = avatarEl(id);
          const pip = document.createElement("span");
          pip.className = "pip";
          av.appendChild(pip);
          const text = document.createElement("span");
          text.className = "mate-text";
          const name = document.createElement("b");
          name.textContent = m.name;
          const what = document.createElement("small");
          if (held) what.textContent = `${held.chrome.label} · ${held.th.title || "untitled"}`;
          else what.textContent = `${waiting} waiting on you`;
          text.append(name, what);
          chip.append(av, text);
          box.appendChild(chip);
        });
      }

      let boardRefreshTimer = null;

      function scheduleBoardRefresh() {
        if (boardRefreshTimer || !selectedChannelId) return;
        boardRefreshTimer = setTimeout(() => {
          boardRefreshTimer = null;
          loadThreads(true);
        }, 250);
      }


      // The open thread's header: state, holder, result and review progress.
      // Bumped on every render so an overlapping call (a board refresh landing
      // right after a card click) cannot append its facts a second time.
      let headerGen = 0;

      async function renderThreadHeader() {
        const gen = ++headerGen;
        const th = threadsById.get(selectedThreadId);
        const badgeBox = document.getElementById("thread-badge");
        const facts = document.getElementById("thread-facts");
        badgeBox.replaceChildren();
        facts.replaceChildren();
        document.getElementById("thread-actions").replaceChildren();
        if (!th) return;
        const tid = th.id;
        badgeBox.textContent = sessionChrome(th, lastGates[tid]).label;
        const fact = (label, node) => {
          const f = document.createElement("span");
          f.className = "fact";
          f.append(label, node);
          facts.appendChild(f);
          return f;
        };
        if (th.assignee_id) fact("held by ", personEl(th.assignee_id, { tag: true }));
        if (th.owner_id) fact("owner ", personEl(th.owner_id));
        try {
          const res = await api(uiReadPath(`/threads/${tid}/result`), {
            headers: headers(),
            credentials: "include",
          });
          if (res.ok && gen === headerGen) {
            const r = await res.json();
            if (gen !== headerGen) return;
            const f = fact("result ", renderResult(r.result));
            f.append(" by ", personEl(r.produced_by, { avatar: false }));
          }
        } catch (_e) {
          /* no result yet */
        }
        if (!token() || gen !== headerGen) return;
        try {
          const res = await api(`${base()}/threads/${tid}/review-status`, { headers: headers() });
          if (res.ok && gen === headerGen) {
            const rs = await res.json();
            if (gen !== headerGen) return;
            if (rs.required_count > 0) {
              const s = document.createElement("span");
              s.className = rs.approvals_met ? "ok" : "wait";
              s.textContent = `${rs.approvals}/${rs.required_count} approvals${rs.approvals_met ? " ✓" : ""}`;
              fact("review ", s);
            }
            renderThreadActions(th, rs);
          }
        } catch (_e) {
          /* review status is optional chrome */
        }
      }



      // ---- Results --------------------------------------------------------
      // A result reads as facts: one chip per field, links clickable, and a
      // zero failure/error count marked good. Deeper shapes fall back to
      // compact JSON so nothing is hidden.
      function renderResult(value) {
        const box = document.createElement("span");
        box.className = "result";
        const chip = (k, v) => {
          const kv = document.createElement("span");
          kv.className = "kv";
          if (k !== null) {
            const ks = document.createElement("span");
            ks.className = "k";
            ks.textContent = k.replace(/_/g, " ");
            kv.appendChild(ks);
          }
          const vs = document.createElement("span");
          vs.className = "v";
          if (typeof v === "string" && /^https?:\/\//.test(v)) {
            const a = document.createElement("a");
            a.href = v;
            a.target = "_blank";
            a.rel = "noopener noreferrer";
            a.title = v;
            a.textContent = v.replace(/^https?:\/\//, "").slice(0, 48);
            vs.appendChild(a);
          } else if (v !== null && typeof v === "object") {
            vs.textContent = JSON.stringify(v).slice(0, 80);
          } else {
            vs.textContent = String(v);
          }
          if (v === 0 && k !== null && /fail|error|flak/i.test(k)) vs.classList.add("good");
          if (v === true && k !== null && /pass|ok|green|success/i.test(k)) vs.classList.add("good");
          kv.appendChild(vs);
          box.appendChild(kv);
        };
        if (value === null || value === undefined) {
          chip(null, "none");
        } else if (Array.isArray(value)) {
          chip(null, `${value.length} item${value.length === 1 ? "" : "s"}`);
        } else if (typeof value === "object") {
          const entries = Object.entries(value);
          entries.slice(0, 6).forEach(([k, v]) => chip(k, v));
          if (entries.length > 6) chip(null, `+${entries.length - 6} more`);
          if (!entries.length) chip(null, "{}");
        } else {
          chip(null, value);
        }
        box.title = JSON.stringify(value);
        return box;
      }

      let pendingGateViews = new Map();

      function faviconHref(alert) {
        const dot = alert ? '<circle cx="52" cy="12" r="11" fill="#ea580c" stroke="#fff" stroke-width="3"/>' : "";
        const mark = document.querySelector(".brand-mark").innerHTML.replace(/currentColor/g, "#14532d");
        return "data:image/svg+xml," + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">${mark}${dot}</svg>`);
      }

      function setAttention(n) {
        document.title = n > 0 ? `(${n}) Maidan` : "Maidan";
        document.getElementById("favicon").href = faviconHref(n > 0);
      }


      function selectThread(id, label) {
        selectedThreadId = id;
        syncCollabPanel();
        document.getElementById("thread-id").value = id;
        document.getElementById("thread-context").textContent = label || id;
        // Reset the live marker for the newly-opened thread.
        const live = document.getElementById("live-indicator");
        if (live) {
          live.hidden = true;
          live.classList.remove("on");
        }
        document.getElementById("thread-context").classList.remove("muted");
        document.querySelectorAll("#board .card").forEach((n) => {
          n.classList.toggle("selected", n.dataset.id === id);
        });
        renderThreadHeader();
        loadMessages();
      }


      // ---- Onboarding, Connect an agent, command palette --------------------
      // An empty channel is one sentence and one action. It replaces the
      // lanes. While the first-run card is on screen, that card is the only
      // primary, so Connect stays a ghost.
      function emptyChannelHelp() {
        const box = document.createElement("div");
        box.className = "onboard";
        box.id = "board-onboard";
        const p = document.createElement("p");
        p.textContent = "A task arrives when an agent or a person opens one.";
        const row = document.createElement("div");
        row.className = "row";
        const b = document.createElement("button");
        b.type = "button";
        b.className = document.getElementById("first-run").hidden ? "primary" : "ghost";
        b.setAttribute("data-open-connect", "");
        b.textContent = "Connect an agent";
        b.onclick = openConnect;
        row.append(b);
        box.append(p, row);
        return box;
      }


      async function loadChannels() {
        const list = document.getElementById("channel-list");
        const aside = document.querySelector("aside");
        if (!wid()) {
          aside.hidden = false;
          renderState(list, "Set a workspace ID to load channels.");
          return;
        }
        setLoading(list, "Loading channels…");
        persist();
        const t = token();
        const url =
          t && !sessionMemberId
            ? `${base()}/workspaces/${wid()}/channels`
            : uiReadPath(`/workspaces/${wid()}/channels`);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) {
            aside.hidden = false;
            renderState(list, await responseError(res, "Could not load channels"), "err");
            return;
          }
          const channels = await res.json();
          // One channel: the name is already the board title, so the sidebar
          // stays off. Two or more, and it comes back, on --soft.
          aside.hidden = channels.length === 1;
          await loadMembers();
          list.innerHTML = "";
          if (!channels.length) {
            renderState(list, "No channels yet. Create one above to open the workspace.");
            return;
          }
          channels.forEach((ch) => {
            const li = document.createElement("li");
            li.dataset.id = ch.id;
            li.innerHTML =
              (ch.private ? "🔒 " : "# ") +
              escapeHtml(ch.name) +
              (ch.topic ? `<span class='private'> · ${escapeHtml(ch.topic)}</span>` : "");
            if (ch.id === selectedChannelId) li.classList.add("selected");
            keyActivates(li);
            li.onclick = () => {
              selectedChannelId = ch.id;
              selectedChannelName = ch.name;
              localStorage.setItem(channelKey, ch.id);
              selectedThreadId = null;
              syncCollabPanel();
              boardSeen = new Map();
              document.getElementById("channel-id").value = ch.id;
              const sc = document.getElementById("search-channel");
              if (!sc.value.trim()) sc.value = ch.id;
              document.getElementById("thread-context").textContent = "none selected";
              document.getElementById("thread-context").classList.add("muted");
              document.getElementById("thread-badge").replaceChildren();
              document.getElementById("thread-facts").replaceChildren();
              // Buttons of the previous task act on that task, so they go too,
              // and a header request still in flight for it must not paint.
              document.getElementById("thread-actions").replaceChildren();
              headerGen++;
              document.querySelectorAll("#channel-list li").forEach((n) => {
                n.classList.toggle("selected", n.dataset.id === ch.id);
              });
              loadThreads();
            };
            list.appendChild(li);
          });
          // Reopen the channel this browser last looked at. A first visit
          // with a single channel opens that board instead of sitting on
          // "pick a channel" under a live Needs you queue.
          const remembered = localStorage.getItem(channelKey);
          const again = remembered && list.querySelector(`li[data-id="${CSS.escape(remembered)}"]`);
          if (!selectedChannelId) {
            if (again) again.click();
            else if (channels.length === 1) list.querySelector("li[data-id]").click();
          }
        } catch (e) {
          aside.hidden = false;
          renderState(list, unreachable(e), "err");
        } finally {
          clearLoading(list);
        }
      }

export { boardChannel, boardNote, boardRects, boardRefreshTimer, boardSeen, boardShownFor, boardState, emptyChannelHelp, faviconHref, fetchPendingGatesByThread, glideCards, headerGen, hydrateRefusals, lastGates, loadChannels, loadThreads, markSeen, markSeenFromFrame, noteRefusalFromFrame, paintRefusal, pendingGateViews, refreshTeamSoon, rememberRefusal, renderBoard, renderResult, renderTeam, renderThreadHeader, scheduleBoardRefresh, selectThread, selectedChannelId, selectedChannelName, selectedThreadId, sessionChrome, setAttention, teamTimer, threadLoadGen, threadsById };
