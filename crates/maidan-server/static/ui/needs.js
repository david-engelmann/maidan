// @ts-check
import { api, apiReadPath, apiWritePath, headers, token, uiReadPath, wid } from "./api.js";
import { fetchPendingGatesByThread, pendingGateViews, renderResult, renderTeam, renderThreadHeader, scheduleBoardRefresh, selectThread, selectedThreadId, setAttention, threadsById } from "./board.js";
import { keyActivates, responseError, setStatus } from "./feedback.js";
import { ago, authorId, personEl } from "./people.js";
import { sessionMemberId } from "./session.js";
import { NY_KINDS, NY_RETRY_MAX_MS, NY_RETRY_MIN_MS } from "./state.js";
import { answerGate } from "./tools.js";



      // ---- Needs you -------------------------------------------------------
      // The decisions agents are waiting on, from the waiting inbox of this member:
      // reviews requested from them and open approval gates. Each row carries
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
        needsYou.forEach((item) => {
          const k = nyKey(item);
          list.appendChild(keep.get(k) || needsYouRow(item));
          keep.delete(k);
        });
        keep.forEach((li) => list.appendChild(li));
        // A queue with nothing in it is one quiet line. A row the human is
        // still using (a change note, a Close task) keeps the box.
        const empty = list.children.length === 0;
        box.classList.toggle("clear", empty);
        document.getElementById("needs-you-quiet").hidden = !empty;
        document.getElementById("needs-you-head").hidden = empty;
        if (empty) box.removeAttribute("aria-labelledby");
        else box.setAttribute("aria-labelledby", "needs-you-title");
        renderTeam([...threadsById.values()]);
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
        kind.textContent = item.kind === "open_gate" ? "Approval" : item.kind === "blocked" ? "Blocked" : "Review";
        const main = document.createElement("div");
        main.className = "ny-main";
        const title = document.createElement("div");
        title.className = "ny-title";
        const th = item.thread_id ? threadsById.get(item.thread_id) : null;
        // A gate row leads with its question: that is what the human answers.
        // A review row leads with the task. (The task of a gate shows as context.)
        const isGate = item.kind === "open_gate";
        title.textContent = isGate ? item.summary : (th && th.title) || item.summary;
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
        if (item.kind === "review_request") {
          sub.appendChild(when);
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
          changes.onclick = () => askForChanges(item, li);
          actions.append(approve, changes);
        } else if (item.kind === "blocked") {
          // A human/gate block: show the reason and note, offer to clear it.
          // The item.summary already carries "title — blocked (reason): note".
          const ctx = document.createElement("span");
          ctx.textContent = item.summary;
          sub.appendChild(ctx);
          sub.appendChild(when);
          const clear = document.createElement("button");
          clear.type = "button";
          clear.className = "primary";
          clear.textContent = "Unblock";
          clear.onclick = async () => {
            let res;
            try {
              res = await api(apiWritePath(`/threads/${item.thread_id}/block`), {
                method: "DELETE",
                headers: headers(true),
                credentials: "include",
              });
            } catch (e) {
              toast(`Could not unblock: ${e.message}`);
              return;
            }
            if (!res.ok) {
              const body = await res.text().catch(() => "");
              toast(`Could not unblock: ${res.status} ${body}`);
              return;
            }
            // Remove from state and re-render; schedule a board refresh so
            // the card loses its blocked marker.
            const idx = needsYou.findIndex((i) => i === item);
            if (idx >= 0) needsYou.splice(idx, 1);
            renderNeedsYou();
            if (typeof scheduleBoardRefresh === "function") scheduleBoardRefresh();
          };
          actions.append(clear);
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
              buttons.forEach((x) => (x.disabled = true));
              const out = await answerGate(item.gate_id, action, v.request_state);
              if (!out.ok) {
                const row = rowOnScreen(item, li);
                const live = row.querySelectorAll(".ny-actions button");
                (live.length ? live : buttons).forEach((x) => (x.disabled = false));
                return showRowError(row, out.why);
              }
              dropRow(item, li);
            };
            actions.appendChild(b);
          }
        }
        return li;
      }

      // Who handed the work off and what they reported, from the result.
      async function fillReviewContext(tid, sub, before) {
        try {
          const res = await api(uiReadPath(`/threads/${tid}/result`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) return;
          const r = await res.json();
          const who = personEl(r.produced_by);
          sub.insertBefore(who, before);
          sub.insertBefore(renderResult(r.result), before);
        } catch (_e) {
          /* the title alone still identifies the review */
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
      async function submitReview(tid, decision, note) {
        if (!token() && !sessionMemberId) {
          return { ok: false, why: "Sign in to approve." };
        }
        let res;
        try {
          res = await api(apiWritePath(`/threads/${tid}/reviews`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify(note ? { decision, note } : { decision }),
          });
        } catch (e) {
          return { ok: false, why: "Review not recorded: could not reach the server. Check the connection and try again." };
        }
        if (!res.ok) return { ok: false, why: await responseError(res, "Review not recorded") };
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

      async function closeThread(tid) {
        let res;
        try {
          res = await api(apiWritePath(`/threads/${tid}`), {
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

      async function approveFromInbox(item, li) {
        const actions = li.querySelector(".ny-actions");
        actions.querySelectorAll("button").forEach((b) => (b.disabled = true));
        const out = await submitReview(item.thread_id, "approve");
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
            close.disabled = true;
            const done = await closeThread(item.thread_id);
            if (!done.ok) {
              close.disabled = false;
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

      function askForChanges(item, li) {
        if (li.querySelector(".ny-note")) return;
        const row = document.createElement("div");
        row.className = "ny-note";
        const input = document.createElement("input");
        input.placeholder = "What should change? The agent reads this.";
        input.setAttribute("aria-label", "Change request note");
        const send = document.createElement("button");
        send.type = "button";
        send.className = "primary";
        send.textContent = "Send back";
        // The note is the decision for this row now, so Approve stops being the
        // filled button until the note is dismissed.
        li.querySelectorAll(".ny-actions button.primary").forEach((b) => {
          b.classList.remove("primary");
          b.dataset.restorePrimary = "1";
        });
        const go = async () => {
          if (send.disabled) return;
          send.disabled = true;
          const out = await submitReview(item.thread_id, "request_changes", input.value.trim());
          if (!out.ok) {
            send.disabled = false;
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
        const mine = needsYou.find((i) => i.kind === "review_request" && i.thread_id === th.id);
        if (mine) {
          const approve = document.createElement("button");
          approve.type = "button";
          approve.className = "primary";
          approve.textContent = "Approve";
          approve.onclick = async () => {
            approve.disabled = true;
            const out = await submitReview(th.id, "approve");
            if (!out.ok) {
              approve.disabled = false;
              return setStatus(out.why, "err");
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
            close.disabled = true;
            const out = await closeThread(th.id);
            if (!out.ok) {
              close.disabled = false;
              return setStatus(out.why, "err");
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

export { approveFromInbox, askForChanges, closeThread, dropRow, fillReviewContext, loadNeedsYou, needsYou, needsYouGen, needsYouRow, nyKey, renderNeedsYou, renderThreadActions, reviewStatus, rowOnScreen, showRowError, submitReview, syncCollabPanel };
