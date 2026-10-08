// @ts-check
import { api, apiReadPath, apiWritePath, headers, token, uiReadPath, wid, writeApi } from "./api.js";
import { formatBytes } from "./artifacts.js";
import { fetchPendingGatesByThread, pendingGateViews, renderResult, renderTeam, renderThreadHeader, scheduleBoardRefresh, selectThread, selectedThreadId, setAttention, threadsById } from "./board.js";
import { keyActivates, responseError, showError } from "./feedback.js";
import { ago, authorId, personEl } from "./people.js";
import { sessionMemberId } from "./session.js";
import { NY_KINDS, NY_RETRY_MAX_MS, NY_RETRY_MIN_MS } from "./state.js";
import { answerGate } from "./tools.js";



      // ---- Needs you -------------------------------------------------------
      // The decisions and actions agents are waiting on, from the waiting inbox
      // of this member: reviews requested from them, open approval gates, and
      // tasks blocked until a person clears them. Each row carries
      // its own buttons. The count also lands in the tab title and favicon, so
      // a waiting agent is visible from another tab.
      let needsYou = [];

      let needsYouGen = 0;

      // When the last good load landed, so a stale queue can say how stale.
      let needsYouLoadedAt = null;

      let needsYouRetryTimer = null;

      let needsYouRetryMs = NY_RETRY_MIN_MS;

      async function loadNeedsYou() {
        const gen = ++needsYouGen;
        const box = document.getElementById("needs-you");
        const me = authorId();
        if (!me || !wid()) {
          stopNeedsYouRetry();
          box.hidden = true;
          setAttention(0);
          return;
        }
        let items = [];
        try {
          const res = await api(uiReadPath(`/members/${me}/waiting`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            // A refused load is not an empty queue. The rows and the count in
            // the tab title stay as last seen, under a sentence naming the fix.
            const why = await responseError(res, "Could not load what is waiting on you");
            if (gen !== needsYouGen) return;
            stopNeedsYouRetry();
            showNeedsYouTrouble(why, "err");
            return;
          }
          items = (await res.json()).items.filter((i) => NY_KINDS.has(i.kind));
          // The request an agent just handed over is the obvious row. Older
          // ones stay, behind it, so a stale review is not the first button.
          items.sort((a, b) => {
            const ag = nyGroup(a.kind);
            const bg = nyGroup(b.kind);
            if (ag.order !== bg.order) return ag.order - bg.order;
            const ar = a.kind === "review_request";
            const br = b.kind === "review_request";
            if (ar && br) return String(b.since).localeCompare(String(a.since));
            if (ar) return -1;
            if (br) return 1;
            return 0;
          });
          if (items.length) items[0].primary = true;
          // Gate rows show the requester and answer with the state of the gate, both
          // from the gate views. The board fills them, but a first visit with no
          // channel open has not loaded the board yet.
          if (items.some((i) => i.gate_id && !pendingGateViews.has(i.gate_id))) {
            await fetchPendingGatesByThread();
          }
        } catch (_e) {
          if (gen !== needsYouGen) return;
          // The server was not reached, so nothing is known to have changed:
          // the rows stay, marked stale, and the load retries on its own.
          const since = needsYouLoadedAt
            ? `Stale since ${needsYouLoadedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}: could not reach the server. `
            : "Could not reach the server. ";
          showNeedsYouTrouble(`${since}Reconnecting…`, "stale");
          scheduleNeedsYouRetry();
          return;
        }
        if (gen !== needsYouGen) return;
        stopNeedsYouRetry();
        needsYouLoadedAt = new Date();
        needsYou = items;
        renderNeedsYou();
      }

      function scheduleNeedsYouRetry() {
        clearTimeout(needsYouRetryTimer);
        needsYouRetryTimer = setTimeout(loadNeedsYou, needsYouRetryMs);
        needsYouRetryMs = Math.min(needsYouRetryMs * 2, NY_RETRY_MAX_MS);
      }

      function stopNeedsYouRetry() {
        clearTimeout(needsYouRetryTimer);
        needsYouRetryTimer = null;
        needsYouRetryMs = NY_RETRY_MIN_MS;
      }

      // The panel stays up with its head, so a failure never reads as
      // "nothing is waiting on you".
      function showNeedsYouTrouble(message, cls) {
        const box = document.getElementById("needs-you");
        const state = document.getElementById("needs-you-state");
        state.textContent = message;
        state.className = `ny-state ${cls}`;
        state.hidden = false;
        box.hidden = false;
        box.classList.remove("clear");
        document.getElementById("needs-you-quiet").hidden = true;
        document.getElementById("needs-you-head").hidden = false;
        box.setAttribute("aria-labelledby", "needs-you-title");
      }

      function renderNeedsYou() {
        const box = document.getElementById("needs-you");
        const list = document.getElementById("needs-you-list");
        const count = document.getElementById("needs-you-count");
        const hint = document.getElementById("needs-you-hint");
        box.hidden = false;
        document.getElementById("needs-you-state").hidden = true;
        // A row the human is using (typing a change note, or approved and
        // about to close) survives a reload, even if its request has left the
        // inbox: rebuilding it would drop the note or the Close task button.
        const keep = new Map();
        for (const li of list.children) {
          const note = li.querySelector(".ny-note input");
          const inUse = li.classList.contains("in-use") || li.contains(document.activeElement) || (note && note.value.trim());
          // A row showing an error survives a reload. Rebuilding it drops the sentence.
          const erred = li.querySelector(".ny-err");
          if (!li.classList.contains("leaving") && (inUse || erred)) {
            keep.set(li.dataset.key, li);
          }
        }
        list.replaceChildren();
        const n = needsYou.length;
        count.textContent = n ? String(n) : "";
        hint.textContent = n
          ? (n === 1
              ? "An agent is waiting on your decision."
              : "Agents are waiting on your decision. The latest request is first.")
          : "";
        setAttention(n);
        // The queue splits into what needs a decision, an action and an
        // answer. With one kind of wait there is nothing to tell apart, so the
        // headings show only when there are two or more.
        const headed = new Set(needsYou.map((i) => nyGroup(i.kind).order)).size > 1;
        let group = null;
        needsYou.forEach((item) => {
          const g = nyGroup(item.kind);
          if (headed && g !== group) {
            const head = document.createElement("li");
            head.className = "ny-group";
            head.setAttribute("role", "presentation");
            head.textContent = g.label;
            list.appendChild(head);
            group = g;
          }
          const k = nyKey(item);
          list.appendChild(keep.get(k) || needsYouRow(item));
          keep.delete(k);
        });
        keep.forEach((li) => list.appendChild(li));
        // A queue with nothing in it is one quiet line. A row the human is
        // still using (a change note, a Close task) keeps the box.
        const empty = list.querySelector(".ny-item") === null;
        box.classList.toggle("clear", empty);
        document.getElementById("needs-you-quiet").hidden = !empty;
        document.getElementById("needs-you-head").hidden = empty;
        if (empty) box.removeAttribute("aria-labelledby");
        else box.setAttribute("aria-labelledby", "needs-you-title");
        renderTeam([...threadsById.values()]);
      }

      const NY_GROUPS = {
        decision: { order: 0, label: "Needs your decision" },
        action: { order: 1, label: "Needs your action" },
        question: { order: 2, label: "An agent asked" },
      };

      function nyGroup(kind) {
        if (kind === "blocked") return NY_GROUPS.action;
        if (kind === "question") return NY_GROUPS.question;
        return NY_GROUPS.decision;
      }

      function nyKey(item) {
        return `${item.kind}:${item.thread_id || ""}:${item.gate_id || ""}`;
      }

      function needsYouRow(item) {
        const li = document.createElement("li");
        li.className = "ny-item";
        li.dataset.key = nyKey(item);
        li.dataset.kind = item.kind;
        if (item.thread_id) li.dataset.threadId = item.thread_id;
        if (item.primary) li.classList.add("ny-primary");
        const kind = document.createElement("span");
        kind.className = `ny-kind${item.kind === "open_gate" ? " gate" : ""}`;
        kind.textContent = { open_gate: "Approval", blocked: "Blocked", question: "Question" }[item.kind] || "Review";
        const main = document.createElement("div");
        main.className = "ny-main";
        const title = document.createElement("div");
        title.className = "ny-title";
        const th = item.thread_id ? threadsById.get(item.thread_id) : null;
        // A gate row leads with its question: that is what the human answers.
        // A review row leads with the task. (The task of a gate shows as context.)
        const isGate = item.kind === "open_gate";
        const block = item.kind === "blocked" ? splitBlockSummary(item.summary) : null;
        const asked = item.kind === "question" ? splitQuestionSummary(item) : null;
        title.textContent = isGate
          ? item.summary
          : (th && th.title) || (block && block.title) || (asked && asked.title) || item.summary;
        if (item.thread_id) {
          title.onclick = () => selectThread(item.thread_id, (th && th.title) || title.textContent);
          keyActivates(title, "link"); // it opens the task
        }
        const sub = document.createElement("div");
        sub.className = "ny-sub";
        const when = document.createElement("span");
        when.textContent = `waiting ${ago(item.since).replace(" ago", "")}`.replace("waiting just now", "just now");
        main.append(title, sub);
        const actions = document.createElement("div");
        actions.className = "ny-actions";
        li.append(kind, main, actions);
        if (item.kind === "review_request" || item.kind === "unassigned_review") {
          sub.appendChild(when);
          if (item.kind === "unassigned_review") {
            const none = document.createElement("span");
            none.className = "ny-unnamed";
            none.textContent = "no reviewer named";
            sub.insertBefore(none, when);
          }
          fillReviewContext(item.thread_id, sub, when);
          const approve = document.createElement("button");
          approve.type = "button";
          approve.className = "primary";
          approve.textContent = "Approve";
          approve.onclick = () => approveFromInbox(item, li);
          const changes = document.createElement("button");
          changes.type = "button";
          changes.className = "ghost";
          changes.textContent = "Request changes";
          changes.onclick = () => askForNote(item, li, "request_changes");
          const approveNoted = document.createElement("button");
          approveNoted.type = "button";
          approveNoted.className = "ghost";
          approveNoted.textContent = "Approve with note";
          approveNoted.onclick = () => askForNote(item, li, "approve");
          actions.append(approve, changes, approveNoted);
          // The approval names what this row showed: the packet as it stood
          // when the row was drawn. Until the row has it there is nothing to
          // bind an approval to, so both approve buttons wait for it, and stay
          // off while the evidence says why it is missing. Request changes
          // needs no packet.
          for (const b of [approve, approveNoted]) {
            b.disabled = true;
            b.dataset.needsRoot = "1";
          }
          const evidence = document.createElement("div");
          evidence.className = "ny-evidence";
          main.appendChild(evidence);
          loadEvidence(item.thread_id, li, evidence);
          if (item.kind === "unassigned_review") ownerActions(item, li, sub, [approve, approveNoted], changes);
        } else if (item.kind === "blocked") {
          // A human/gate block: show the reason and note, offer to clear it.
          // The title is already the row's title, so it is not repeated here.
          const ctx = document.createElement("span");
          ctx.textContent = block ? block.why : item.summary;
          sub.appendChild(ctx);
          sub.appendChild(when);
          const clear = document.createElement("button");
          clear.type = "button";
          clear.className = "primary";
          clear.textContent = "Unblock";
          clear.onclick = async () => {
            let res;
            try {
              res = await writeApi(clear, apiWritePath(`/threads/${item.thread_id}/block`), {
                method: "DELETE",
                headers: headers(true),
                credentials: "include",
              });
            } catch (_e) {
              showRowError(rowOnScreen(item, li), "Could not unblock: the server did not answer. Try again.");
              return;
            }
            if (!res.ok) {
              showRowError(rowOnScreen(item, li), await responseError(res, "Could not unblock"));
              return;
            }
            // dropRow marks the row as leaving. A row that showed an error is
            // otherwise kept across a re-render, so after a failed try and a
            // good one the unblocked row stayed up under its old error.
            dropRow(item, rowOnScreen(item, li));
            // The card loses its blocked marker on the next board load.
            scheduleBoardRefresh();
          };
          actions.append(clear);
        } else if (item.kind === "question") {
          // The agent's question is what the human answers; a reply in the
          // thread clears it, so Answer opens the thread at the composer.
          const q = document.createElement("span");
          q.className = "ny-question";
          q.textContent = asked.question;
          sub.append(q, when);
          const answer = document.createElement("button");
          answer.type = "button";
          answer.className = "primary";
          answer.textContent = "Answer";
          answer.onclick = () => {
            selectThread(item.thread_id, (th && th.title) || asked.title);
            const compose = document.getElementById("compose-body");
            if (compose) compose.focus();
          };
          actions.append(answer);
        } else {
          const view = pendingGateViews.get(item.gate_id);
          if (view && view.gate.requested_by) sub.append("asked by ", personEl(view.gate.requested_by));
          if (th && th.title) {
            const on = document.createElement("span");
            on.className = "ny-on";
            on.textContent = `on ${th.title}`;
            sub.appendChild(on);
          }
          sub.appendChild(when);
          for (const [action, label, cls] of [["accept", "Approve", "primary"], ["decline", "Decline", "ghost"]]) {
            const b = document.createElement("button");
            b.type = "button";
            b.textContent = label;
            if (cls) b.className = cls;
            b.onclick = async () => {
              let v = pendingGateViews.get(item.gate_id);
              if (!v) {
                await fetchPendingGatesByThread();
                v = pendingGateViews.get(item.gate_id);
              }
              if (!v) return showRowError(li, "This gate is no longer pending.");
              const buttons = actions.querySelectorAll("button");
              const out = await answerGate(item.gate_id, action, v.request_state, buttons);
              if (!out.ok) {
                return showRowError(rowOnScreen(item, li), out.why);
              }
              dropRow(item, li);
            };
            actions.appendChild(b);
          }
        }
        return li;
      }

      // A blocked item's summary is "title — blocked (reason): note", or
      // "title — blocked: reason" with no note. The title leads the row, and
      // the rest says why. A title of a channel not loaded yet comes from here.
      function splitBlockSummary(summary) {
        const s = String(summary || "");
        const at = s.indexOf(" — blocked");
        if (at < 0) return { title: s, why: s };
        return { title: s.slice(0, at), why: s.slice(at + 3) };
      }

      // A question's summary is "title: question", and `detail` is the
      // question alone. A title can hold ": " itself, so the title is what is
      // left once the known question is taken off the end.
      function splitQuestionSummary(item) {
        const s = String(item.summary || "");
        const question = String(item.detail || "");
        const tail = `: ${question}`;
        if (question && s.endsWith(tail)) return { title: s.slice(0, -tail.length), question };
        return { title: s, question: question || s };
      }

      // Who handed the work off and what they reported, from the result. A
      // review can start with no result on a thread with no review gate; the
      // row says so, so nobody approves an empty hand-off without knowing.
      async function fillReviewContext(tid, sub, before) {
        try {
          const res = await api(uiReadPath(`/threads/${tid}/result`), {
            headers: headers(),
            credentials: "include",
          });
          if (res.status === 404) {
            const warn = document.createElement("span");
            warn.className = "ny-warn";
            warn.textContent = "No result was posted";
            sub.insertBefore(warn, before);
            return;
          }
          if (!res.ok) return;
          const r = await res.json();
          const who = personEl(r.produced_by);
          sub.insertBefore(who, before);
          sub.insertBefore(renderResult(r.result), before);
        } catch (_e) {
          /* the title alone still identifies the review */
        }
      }

      // The owner hears about a review nobody was named for, but its own
      // approval does not count (separation of duties). With no review
      // requirement it may close the task itself, which the board then shows
      // as closed without review. With one, sending the work back is the
      // owner's move, and naming a reviewer is how it gets approved.
      async function ownerActions(item, li, sub, approvals, changes) {
        if (!(await viewerOwns(item.thread_id))) return;
        approvals.forEach((b) => b.remove());
        const rs = await reviewStatus(item.thread_id);
        if (rs && rs.required_count === 0) {
          const close = document.createElement("button");
          close.type = "button";
          close.className = "primary";
          close.textContent = "Close without review";
          close.onclick = async () => {
            const done = await closeThread(item.thread_id, close);
            if (!done.ok) {
              return showRowError(rowOnScreen(item, li), done.why);
            }
            dropRow(item, li);
            scheduleBoardRefresh();
          };
          changes.before(close);
          return;
        }
        changes.className = "primary";
        const why = document.createElement("span");
        why.className = "ny-unnamed";
        why.textContent = "your approval does not count: name a reviewer";
        sub.appendChild(why);
      }

      async function viewerOwns(tid) {
        const me = authorId();
        const known = threadsById.get(tid);
        if (known) return Boolean(me) && known.owner_id === me;
        try {
          const res = await api(uiReadPath(`/threads/${tid}`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) return false;
          return Boolean(me) && (await res.json()).owner_id === me;
        } catch (_e) {
          return false;
        }
      }

      // The row captured at click time may have been replaced while the
      // request was in flight. Paint on the row that is on screen.
      function rowOnScreen(item, li) {
        const key = (li && li.dataset && li.dataset.key) || nyKey(item);
        if (!key) return li;
        const rows = document.querySelectorAll("#needs-you-list .ny-item");
        for (const row of rows) {
          if (row.dataset.key === key) return row;
        }
        return li;
      }

      function showRowError(li, msg) {
        let el = li.querySelector(".ny-err");
        if (!el) {
          el = document.createElement("div");
          el.className = "ny-err";
          li.appendChild(el);
        }
        el.textContent = msg;
      }

      function dropRow(item, li) {
        li.classList.add("leaving");
        setTimeout(() => {
          // By identity: a reload may have replaced the item object.
          needsYou = needsYou.filter((i) => nyKey(i) !== nyKey(item));
          renderNeedsYou();
        }, 380);
      }

      // Reviews and closes are thread transitions. An OIDC session carries
      // thread:transition; a token uses its own grant. The store still refuses
      // a self-approval, including one made with a borrowed token.
      // What the thread was handed to review with, or null before any review.
      async function reviewPacket(tid) {
        const read = await readReviewPacket(tid);
        return read.packet || null;
      }

      // The packet as a result the row can tell apart: a packet, nothing
      // handed over yet (404), or a read that failed and says why.
      async function readReviewPacket(tid) {
        let res;
        try {
          res = await api(apiReadPath(`/threads/${tid}/review-packet`), {
            headers: headers(),
            credentials: "include",
          });
        } catch (_e) {
          return { why: "Could not load the evidence: the server did not answer" };
        }
        if (res.status === 404) return { none: true };
        if (!res.ok) return { why: await responseError(res, "Could not load the evidence") };
        try {
          return { packet: await res.json() };
        } catch (_e) {
          return { why: "Could not load the evidence: the server sent something unreadable" };
        }
      }

      // Who linked each artifact to the task and when, keyed by hash. Both
      // trees serve it, behind the thread check the packet read has. null: the
      // read failed, so nothing is known about the links and no row says one
      // was dropped.
      async function threadLinks(tid) {
        try {
          const res = await api(apiReadPath(`/threads/${tid}/artifacts`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) return null;
          const body = await res.json();
          if (!Array.isArray(body)) return null;
          const links = new Map();
          for (const link of body) {
            if (link && link.sha256) links.set(link.sha256, link);
          }
          return links;
        } catch (_e) {
          return null;
        }
      }

      // The task's standing verdicts, so a page loaded after one was given
      // still names who gave it. null: the read failed; the row shows only
      // what this page saw.
      async function threadReviews(tid) {
        try {
          const res = await api(apiReadPath(`/threads/${tid}/reviews`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) return null;
          const reviews = await res.json();
          return Array.isArray(reviews) ? reviews : null;
        } catch (_e) {
          return null;
        }
      }

      // One artifact's metadata, or why it could not be read. Not cached: a
      // failed read is retried the next time the row is drawn.
      async function evidenceMeta(sha) {
        try {
          const res = await api(uiReadPath(`/artifacts/${encodeURIComponent(sha)}/meta`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) return { why: await responseError(res, "Could not load this artifact's details") };
          return { meta: await res.json() };
        } catch (_e) {
          return { why: "Could not load this artifact's details: the server did not answer" };
        }
      }

      function shortHash(h) {
        return `${String(h || "").slice(0, 12)}…`;
      }

      function hashEl(h, cls) {
        const code = document.createElement("code");
        code.className = cls;
        code.textContent = shortHash(h);
        code.title = h || "";
        return code;
      }

      // The evidence a review row is approving: the packet the thread was
      // handed to review with, the result's hash and who produced it, then
      // each linked artifact. The root it shows is the root Approve sends.
      // One artifact that cannot be read says so in its own line; the rest
      // of the evidence still shows.
      async function loadEvidence(tid, li, box) {
        box.replaceChildren();
        box.dataset.state = "loading";
        box.setAttribute("aria-busy", "true");
        // A verdict can land (the page's own decision, or a live frame) while
        // this read is in flight. Anything recorded at or after this instant
        // is newer than the snapshot and noteReviews keeps it.
        const reviewsReadAt = Date.now();
        const [read, reviews] = await Promise.all([readReviewPacket(tid), threadReviews(tid)]);
        box.removeAttribute("aria-busy");
        if (reviews) noteReviews(tid, reviews, reviewsReadAt);
        if (read.why) {
          delete li.dataset.evidenceRoot;
          box.dataset.state = "error";
          const err = document.createElement("span");
          err.className = "ny-ev-err";
          err.setAttribute("role", "alert");
          err.textContent = `${read.why}.`;
          const retry = document.createElement("button");
          retry.type = "button";
          retry.className = "ghost ny-ev-retry";
          retry.textContent = "Retry";
          retry.onclick = () => loadEvidence(tid, li, box);
          box.append(err, retry);
          renderDecider(li);
          return;
        }
        if (read.none) {
          delete li.dataset.evidenceRoot;
          box.dataset.state = "none";
          const none = document.createElement("span");
          none.className = "ny-ev-empty";
          none.textContent = "Nothing was handed to review yet, so there is no evidence to show.";
          box.appendChild(none);
          renderDecider(li);
          return;
        }
        const packet = read.packet;
        const manifest = packet.manifest || {};
        const shas = Array.isArray(manifest.artifacts) ? manifest.artifacts : [];
        // Each item's tier was judged by the server at the hand-off and is in
        // the root. A packet from before tiers has none, and shows none.
        const attestations = Array.isArray(manifest.attestations) ? manifest.attestations : [];
        const ofKind = (k) => attestations.filter((a) => a && a.kind === k);
        const artifactTiers = ofKind("artifact");
        const gates = ofKind("land_gate");
        li.dataset.evidenceRoot = packet.evidence_root;
        // The approve buttons wait for a root; a retry that loads one turns them on.
        if (packet.evidence_root) {
          li.querySelectorAll("button[data-needs-root]").forEach((b) => {
            if (b instanceof HTMLButtonElement) b.disabled = false;
          });
        }
        const head = document.createElement("div");
        head.className = "ny-ev-head";
        const root = hashEl(packet.evidence_root, "ny-ev-root");
        root.dataset.root = packet.evidence_root;
        head.append("Evidence ", root);
        box.appendChild(head);
        if (packet.self_reported_only === true) {
          // The server decides when to warn; the page only says so.
          const warn = document.createElement("div");
          warn.className = "ny-ev-warn";
          warn.setAttribute("role", "note");
          warn.textContent = "Self-reported only: all of this evidence comes from the task's own workers. Nothing was attached by anyone else or verified by a land gate.";
          box.appendChild(warn);
        }
        if (!manifest.result && !shas.length && !gates.length) {
          box.dataset.state = "empty";
          const empty = document.createElement("span");
          empty.className = "ny-ev-empty";
          empty.textContent = "No evidence was handed over: no result and no linked artifacts.";
          box.appendChild(empty);
          renderDecider(li);
          return;
        }
        box.dataset.state = "ready";
        const list = document.createElement("ul");
        list.className = "ny-ev-list";
        box.appendChild(list);
        if (manifest.result) {
          const r = document.createElement("li");
          r.className = "ny-ev-item";
          r.dataset.ev = "result";
          const kind = document.createElement("span");
          kind.className = "ny-ev-kind";
          kind.textContent = "result";
          r.append(kind, hashEl(manifest.result.sha256, "ny-ev-hash"), " produced by ", personEl(manifest.result.produced_by, { avatar: false, tag: true }));
          appendTier(r, ofKind("result")[0]);
          list.appendChild(r);
        }
        const rows = shas.map((sha) => {
          const a = document.createElement("li");
          a.className = "ny-ev-item";
          a.dataset.ev = "artifact";
          a.dataset.sha = sha;
          a.setAttribute("aria-busy", "true");
          a.appendChild(hashEl(sha, "ny-ev-hash"));
          list.appendChild(a);
          return a;
        });
        for (const gate of gates) {
          const g = document.createElement("li");
          g.className = "ny-ev-item";
          g.dataset.ev = "land_gate";
          const kind = document.createElement("span");
          kind.className = "ny-ev-kind";
          kind.textContent = "land-gate pass";
          g.appendChild(kind);
          if (gate.sha256) g.append(hashEl(gate.sha256, "ny-ev-hash"));
          g.append(" recorded by ", personEl(gate.attested_by, { avatar: false, tag: true }));
          appendTier(g, gate);
          list.appendChild(g);
        }
        const [links, metas] = await Promise.all([threadLinks(tid), Promise.all(shas.map(evidenceMeta))]);
        shas.forEach((sha, i) => {
          renderEvidenceArtifact(rows[i], sha, metas[i], links);
          appendTier(rows[i], artifactTiers.find((a) => a.sha256 === sha) || artifactTiers[i]);
        });
        renderDecider(li);
      }

      const TIER_TEXT = {
        verified: ["verified", "A land-gate pass the close gate accepts"],
        attached: ["attached", "Linked by a member who never worked the task"],
        self_reported: ["self-reported", "The work's own account: from a member who worked the task"],
      };

      // One item's attestation tier, as the server recorded it. An unknown
      // tier is shown by its wire name rather than dropped.
      function appendTier(row, attestation) {
        if (!attestation || !attestation.tier) return;
        const [text, title] = TIER_TEXT[attestation.tier] || [String(attestation.tier), ""];
        const tier = document.createElement("span");
        tier.className = "ny-ev-tier";
        tier.dataset.tier = String(attestation.tier);
        tier.textContent = text;
        if (title) tier.title = title;
        row.appendChild(tier);
      }

      // Who linked the bytes to the task and when. A hash the packet pinned
      // that the task no longer links is flagged: approving this packet is
      // refused until it is handed over again. Without the links, who
      // uploaded the bytes.
      function renderEvidenceArtifact(row, sha, read, links) {
        row.removeAttribute("aria-busy");
        row.replaceChildren();
        if (read.why) {
          row.classList.add("ny-ev-failed");
          const err = document.createElement("span");
          err.className = "ny-ev-err";
          err.textContent = `${read.why}.`;
          row.append(hashEl(sha, "ny-ev-hash"), " ", err);
          return;
        }
        const meta = read.meta;
        const kind = document.createElement("span");
        kind.className = "ny-ev-kind";
        kind.textContent = String(meta.kind || "artifact").replace(/_/g, " ");
        const name = document.createElement("span");
        name.className = "ny-ev-name";
        name.textContent = meta.filename || "unnamed";
        const size = document.createElement("span");
        size.className = "ny-ev-size";
        size.textContent = formatBytes(Number(meta.size_bytes));
        row.append(kind, name, size, hashEl(sha, "ny-ev-hash"));
        const link = links && links.get(sha);
        const by = document.createElement("span");
        by.className = "ny-ev-by";
        if (link) {
          by.append("linked by ", personEl(link.linked_by, { avatar: false, tag: true }), ` ${ago(link.linked_at)}`);
        } else if (links) {
          by.classList.add("ny-warn");
          by.textContent = "no longer linked to the task";
        } else if (meta.uploaded_by) {
          by.append("uploaded by ", personEl(meta.uploaded_by, { avatar: false, tag: true }), ` ${ago(meta.created_at)}`);
        }
        if (by.childNodes.length) row.appendChild(by);
      }

      // Who decided, for each task: from the task's reviews when its row
      // loads, the review a decision returned, or a review_submitted event
      // on the socket. Keyed by task, then reviewer, since a task can need
      // two.
      const decisions = new Map();

      // A server time as milliseconds, 0 when absent or unreadable.
      function serverTime(value) {
        const ms = value ? Date.parse(value) : NaN;
        return Number.isFinite(ms) ? ms : 0;
      }

      // seenAt is this page's clock when the verdict reached it; at is the
      // server's time for it (a review's updated_at, a frame's occurred_at).
      function decisionOf(review, seenAt) {
        return {
          reviewer_id: review.reviewer_id,
          actor_id: review.actor_id || null,
          decision: review.decision,
          evidence_root: review.evidence_root || null,
          seenAt: seenAt || 0,
          at: serverTime(review.updated_at || review.occurred_at || review.created_at),
        };
      }

      // The reviews the server holds replace what this page knew of the task.
      // A verdict that reached the page while the read was in flight
      // (readAt) may be newer than the snapshot: it stays unless the snapshot
      // holds a later row for that reviewer, a dismissal included. A
      // dismissed review is no verdict.
      function noteReviews(tid, reviews, readAt) {
        const byReviewer = new Map();
        const savedAt = new Map();
        for (const review of reviews) {
          if (!review || !review.reviewer_id) continue;
          savedAt.set(
            review.reviewer_id,
            Math.max(serverTime(review.updated_at), serverTime(review.dismissed_at)),
          );
          if (review.dismissed_at) continue;
          byReviewer.set(review.reviewer_id, decisionOf(review));
        }
        const prior = decisions.get(tid);
        if (prior) {
          for (const [id, known] of prior) {
            if (known.seenAt < readAt) continue;
            const saved = savedAt.get(id);
            if (saved === undefined || !known.at || known.at > saved) byReviewer.set(id, known);
          }
        }
        decisions.set(tid, byReviewer);
      }

      function noteDecision(tid, review) {
        if (!tid || !review || !review.reviewer_id) return;
        const byReviewer = decisions.get(tid) || new Map();
        byReviewer.set(review.reviewer_id, decisionOf(review, Date.now()));
        decisions.set(tid, byReviewer);
        for (const row of document.querySelectorAll("#needs-you-list .ny-item")) {
          if (row instanceof HTMLElement && row.dataset.threadId === tid) renderDecider(row);
        }
      }

      // A live frame for a verdict on a task with a row here names who gave it.
      function noteDecisionFrame(frame) {
        if (!frame || frame.kind !== "review_submitted") return;
        noteDecision(frame.thread_id, frame);
      }

      function renderDecider(li) {
        const box = li.querySelector(".ny-evidence");
        const byReviewer = decisions.get(li.dataset.threadId);
        if (!box || !byReviewer) return;
        box.querySelectorAll(".ny-ev-decider").forEach((el) => el.remove());
        const shown = li.dataset.evidenceRoot;
        for (const d of byReviewer.values()) {
          // An approval of an earlier hand-off counts no more, so it names
          // no decider of the evidence shown.
          if (d.decision === "approve" && shown && d.evidence_root && d.evidence_root !== shown) continue;
          const line = document.createElement("div");
          line.className = "ny-ev-decider";
          line.dataset.decision = d.decision || "";
          line.dataset.reviewer = d.reviewer_id;
          line.append(
            personEl(d.reviewer_id, { avatar: false }),
            d.decision === "request_changes" ? " requested changes" : " approved",
          );
          if (d.actor_id && d.actor_id !== d.reviewer_id) {
            line.append(", submitted by ", personEl(d.actor_id, { avatar: false }));
          }
          box.appendChild(line);
        }
      }

      // An approval names the evidence it approves: the root of the packet the
      // row showed, or, without one, the packet as it stands when clicked.
      async function evidenceRootFor(tid, decision, shown) {
        if (shown || decision !== "approve") return shown;
        const packet = await reviewPacket(tid);
        return packet && packet.evidence_root;
      }

      async function submitReview(tid, decision, note, button, evidenceRoot) {
        if (!token() && !sessionMemberId) {
          return { ok: false, why: "Sign in to approve." };
        }
        const root = await evidenceRootFor(tid, decision, evidenceRoot);
        if (decision === "approve" && !root) {
          return { ok: false, why: "Nothing was handed to review yet, so there is nothing to approve." };
        }
        const body = { decision, ...(note && { note }), ...(root && { evidence_root: root }) };
        let res;
        try {
          res = await writeApi(button || null, apiWritePath(`/threads/${tid}/reviews`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify(body),
          });
        } catch (e) {
          return { ok: false, why: "Review not recorded: could not reach the server. Check the connection and try again." };
        }
        if (!res.ok) return { ok: false, why: await responseError(res, "Review not recorded") };
        let review = null;
        try {
          review = await res.json();
        } catch (_e) {
          /* recorded; the decider line waits for the live event */
        }
        noteDecision(tid, review);
        return { ok: true };
      }

      async function reviewStatus(tid) {
        try {
          const res = await api(apiReadPath(`/threads/${tid}/review-status`), {
            headers: headers(),
            credentials: "include",
          });
          return res.ok ? await res.json() : null;
        } catch (_e) {
          return null;
        }
      }

      async function closeThread(tid, button) {
        let res;
        try {
          res = await writeApi(button || null, apiWritePath(`/threads/${tid}`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ action: "close" }),
          });
        } catch (e) {
          return { ok: false, why: "Not closed: could not reach the server. Check the connection and try again." };
        }
        if (!res.ok) return { ok: false, why: await responseError(res, "Not closed") };
        return { ok: true };
      }

      async function approveFromInbox(item, li, reviewNote) {
        // Only the packet this row showed is approved here; a row without one
        // says so rather than approve whatever stands now.
        if (!li.dataset.evidenceRoot) {
          return showRowError(rowOnScreen(item, li), "Nothing was handed to review yet, or it could not be loaded, so there is nothing to approve. Reload and try again.");
        }
        const actions = li.querySelector(".ny-actions");
        const out = await submitReview(
          item.thread_id,
          "approve",
          reviewNote,
          actions.querySelectorAll("button"),
          li.dataset.evidenceRoot,
        );
        const row = rowOnScreen(item, li);
        const liveActions = row.querySelector(".ny-actions");
        if (!out.ok) {
          if (liveActions) liveActions.querySelectorAll("button").forEach((b) => (b.disabled = false));
          return showRowError(row, out.why);
        }
        li.classList.add("in-use");
        const rs = await reviewStatus(item.thread_id);
        actions.replaceChildren();
        const note = document.createElement("span");
        note.className = "done-note";
        note.textContent = rs && rs.required_count
          ? `Approved ✓ ${rs.approvals}/${rs.required_count}`
          : "Approved ✓";
        actions.appendChild(note);
        if (rs && rs.approvals_met) {
          const close = document.createElement("button");
          close.type = "button";
          close.className = "primary";
          close.textContent = "Close task";
          close.onclick = async () => {
            const done = await closeThread(item.thread_id, close);
            if (!done.ok) {
              return showRowError(li, done.why);
            }
            dropRow(item, li);
            scheduleBoardRefresh();
          };
          actions.appendChild(close);
        } else {
          setTimeout(() => dropRow(item, li), 1200);
        }
        if (item.thread_id === selectedThreadId) renderThreadHeader();
      }

      // A note row for a verdict. A change request says what to change, so
      // there is nothing to send until the note does; an approval's note is
      // optional.
      function askForNote(item, li, decision) {
        const open = li.querySelector(".ny-note");
        if (open) {
          if (open.dataset.decision === decision) return;
          open.remove();
        }
        const changes = decision === "request_changes";
        const row = document.createElement("div");
        row.className = "ny-note";
        row.dataset.decision = decision;
        const input = document.createElement("input");
        input.placeholder = changes
          ? "What should change? The agent reads this."
          : "Anything the agent should know? Optional.";
        input.setAttribute("aria-label", changes ? "Change request note" : "Approval note");
        const send = document.createElement("button");
        send.type = "button";
        send.className = "primary";
        send.textContent = changes ? "Send back" : "Approve";
        send.disabled = changes;
        input.oninput = () => {
          send.disabled = changes && !input.value.trim();
        };
        // The note is the decision for this row now, so Approve stops being the
        // filled button until the note is dismissed.
        li.querySelectorAll(".ny-actions button.primary").forEach((b) => {
          b.classList.remove("primary");
          b.dataset.restorePrimary = "1";
        });
        const go = async () => {
          if (send.disabled) return;
          const note = input.value.trim() || undefined;
          if (!changes) {
            row.remove();
            li.querySelectorAll("[data-restore-primary]").forEach((b) => b.classList.add("primary"));
            return approveFromInbox(item, li, note);
          }
          const out = await submitReview(item.thread_id, "request_changes", note, send);
          if (!out.ok) {
            return showRowError(rowOnScreen(item, li), out.why);
          }
          dropRow(item, li);
          scheduleBoardRefresh();
        };
        send.onclick = go;
        input.onkeydown = (e) => {
          if (e.key === "Enter") go();
          if (e.key === "Escape") {
            row.remove();
            li.querySelectorAll("[data-restore-primary]").forEach((b) => b.classList.add("primary"));
          }
        };
        row.append(input, send);
        li.appendChild(row);
        input.focus();
      }

      // Decision buttons for the open thread: approve when a review is
      // requested from me, close once the review requirement is met.
      function renderThreadActions(th, rs) {
        const box = document.getElementById("thread-actions");
        box.replaceChildren();
        if (th.state !== "in_review" || (!token() && !sessionMemberId)) return;
        const mine = needsYou.find(
          (i) =>
            i.thread_id === th.id &&
            (i.kind === "review_request" || (i.kind === "unassigned_review" && th.owner_id !== authorId()))
        );
        if (mine) {
          const approve = document.createElement("button");
          approve.type = "button";
          approve.className = "primary";
          approve.textContent = "Approve";
          approve.onclick = async () => {
            const out = await submitReview(th.id, "approve", undefined, approve);
            if (!out.ok) {
              return showError(out.why);
            }
            await loadNeedsYou();
            renderThreadHeader();
          };
          box.appendChild(approve);
        } else if (rs && rs.required_count > 0 && rs.approvals_met) {
          const close = document.createElement("button");
          close.type = "button";
          close.className = "primary";
          close.textContent = "Close task";
          close.onclick = async () => {
            const out = await closeThread(th.id, close);
            if (!out.ok) {
              return showError(out.why);
            }
            scheduleBoardRefresh();
          };
          box.appendChild(close);
        }
      }


      function syncCollabPanel() {
        // The thread, the composer, and Post stay off the first screen
        // until a card is open.
        document.getElementById("collab-panel").hidden = !selectedThreadId;
      }

export { approveFromInbox, askForNote, closeThread, dropRow, fillReviewContext, loadEvidence, loadNeedsYou, needsYou, needsYouGen, needsYouRow, noteDecisionFrame, nyKey, renderNeedsYou, renderThreadActions, reviewPacket, reviewStatus, rowOnScreen, showRowError, submitReview, syncCollabPanel };
