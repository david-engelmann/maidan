// @ts-check
import { api, apiWritePath, base, headers, persist, requireAuthForWrite, token, uiReadPath } from "./api.js";
import { artifactCard, artifactShasFromMetadata } from "./artifacts.js";
import { selectedThreadId } from "./board.js";
import { clearLoading, renderState, responseError, setLoading, setStatus, unreachable } from "./feedback.js";
import { authorId, avatarEl, personEl } from "./people.js";
import { QUICK_REACTIONS, THREAD_CONTENT_KINDS } from "./state.js";
import { loadMessageEdits } from "./tools.js";


      let pinnedIds = new Set();


      async function loadMessages() {
        const box = document.getElementById("message-list");
        if (!selectedThreadId) {
          renderState(box, "Select a thread to read its messages.");
          return;
        }
        setLoading(box, "Loading messages…");
        persist();
        const url = token()
          ? `${base()}/threads/${selectedThreadId}/messages?limit=50`
          : uiReadPath(`/threads/${selectedThreadId}/messages?limit=50`);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) {
            renderState(box, await responseError(res), "err");
            return;
          }
          await loadPins(selectedThreadId);
          renderMessages(await res.json(), box);
        } catch (e) {
          renderState(box, unreachable(e), "err");
        } finally {
          clearLoading(box);
        }
      }

      let liveRefreshTimer = null;

      function flashLiveIndicator() {
        const el = document.getElementById("live-indicator");
        if (!el) return;
        el.hidden = false;
        el.classList.add("on");
        setTimeout(() => el.classList.remove("on"), 600);
      }

      // Coalesce bursts: at most one reload per window, trailing indicator flash.
      function scheduleLiveRefresh() {
        if (liveRefreshTimer) return;
        liveRefreshTimer = setTimeout(() => {
          liveRefreshTimer = null;
          if (selectedThreadId) {
            loadMessages();
            flashLiveIndicator();
          }
        }, 300);
      }

      // A frame refreshes the open thread only when it names that thread and
      // carries a message/reaction/pin change.
      function liveFrameTargetsOpenThread(frame) {
        return (
          !!selectedThreadId &&
          frame.thread_id === selectedThreadId &&
          THREAD_CONTENT_KINDS.has(frame.kind)
        );
      }


      function renderMessages(messages, box) {
        box.innerHTML = "";
        artifactObjectUrls.forEach((url) => URL.revokeObjectURL(url));
        artifactObjectUrls = [];
        if (!messages.length) {
          renderState(box, "No messages yet. Start the conversation below.");
          return;
        }
        messages.forEach((m) => {
          const div = document.createElement("div");
          div.className = "msg";
          div.dataset.id = m.id;
          div.appendChild(avatarEl(m.author_id, true));
          const meta = document.createElement("div");
          meta.className = "meta";
          meta.appendChild(personEl(m.author_id, { avatar: false, tag: true }));
          const time = document.createElement("span");
          time.className = "time";
          time.title = `${m.id}${m.posted_at ? ` · ${m.posted_at}` : ""}`;
          time.textContent = m.posted_at
            ? new Date(m.posted_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
            : "";
          meta.appendChild(time);
          if (m.edited_at) meta.appendChild(document.createTextNode("edited"));
          const pinned = pinnedIds.has(m.id);
          const pinBtn = document.createElement("button");
          pinBtn.type = "button";
          pinBtn.className = "pin-toggle" + (pinned ? " pinned" : "");
          pinBtn.textContent = pinned ? "📌 pinned" : "📌 pin";
          pinBtn.title = pinned ? "Unpin" : "Pin to thread";
          pinBtn.onclick = (ev) => {
            ev.stopPropagation();
            togglePin(selectedThreadId, m.id, pinned);
          };
          meta.appendChild(pinBtn);
          const body = document.createElement("div");
          body.className = "body";
          body.textContent = m.body;
          div.appendChild(meta);
          div.appendChild(body);
          renderSlashResult(m.metadata, div);
          artifactShasFromMetadata(m.metadata).forEach((sha) => div.appendChild(artifactCard(sha)));
          div.onclick = () => {
            document.getElementById("edit-message-id").value = m.id;
            document.getElementById("edit-message-body").value = m.body;
            loadMessageEdits(m.id);
          };
          const reactions = document.createElement("div");
          reactions.className = "reactions";
          div.appendChild(reactions);
          loadReactions(m.id, reactions);
          box.appendChild(div);
        });
      }


      function renderSlashResult(meta, div) {
        if (!meta || typeof meta !== "object" || !meta.slash_command) return;
        const sc = meta.slash_command;
        const sr = meta.slash_response || {};
        const boxEl = document.createElement("div");
        boxEl.className = "slash-result";
        const head = document.createElement("div");
        head.className = "slash-head";
        head.textContent = `⌘ /${sc.name}${sc.args ? " " + sc.args : ""}`;
        boxEl.appendChild(head);
        const status = document.createElement("div");
        if (sr.ok) {
          status.className = "slash-ok";
          status.textContent = "✓ ok";
        } else if (sr.retrying) {
          status.className = "slash-warn";
          status.textContent = `⟳ retrying${sr.error ? " · " + sr.error : ""}`;
        } else {
          status.className = "slash-err";
          // The failure kind leads: for a WASI handler it is the difference
          // between "shrink your loop" and "fix your crash", and the prose
          // alone does not carry that.
          const kind = sr.error_kind ? `${sr.error_kind} · ` : "";
          const code =
            sr.exit_code !== undefined && sr.exit_code !== null
              ? ` (exit ${sr.exit_code})`
              : "";
          status.textContent = `✗ ${kind}${sr.error || "failed"}${code}`;
        }
        boxEl.appendChild(status);
        if (sr.response !== undefined) {
          const pre = document.createElement("pre");
          pre.className = "slash-response";
          pre.textContent =
            typeof sr.response === "string"
              ? sr.response
              : JSON.stringify(sr.response, null, 2);
          boxEl.appendChild(pre);
        }
        div.appendChild(boxEl);
      }


      async function loadReactions(messageId, el) {
        el.innerHTML = "";
        const url = token()
          ? `${base()}/messages/${messageId}/reactions`
          : uiReadPath(`/messages/${messageId}/reactions`);
        let list = [];
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (res.ok) list = await res.json();
        } catch (e) {
          /* leave reactions empty on error */
        }
        const me = authorId();
        const agg = {};
        list.forEach((r) => {
          const a = agg[r.emoji] || (agg[r.emoji] = { count: 0, mine: false });
          a.count += 1;
          if (me && r.member_id === me) a.mine = true;
        });
        Object.keys(agg).forEach((emoji) => {
          const { count, mine } = agg[emoji];
          const chip = document.createElement("button");
          chip.type = "button";
          chip.className = "reaction-chip" + (mine ? " mine" : "");
          chip.textContent = `${emoji} ${count}`;
          chip.title = mine ? "Remove your reaction" : "React";
          chip.onclick = (ev) => {
            ev.stopPropagation();
            toggleReaction(messageId, emoji, mine, el);
          };
          el.appendChild(chip);
        });
        QUICK_REACTIONS.forEach((emoji) => {
          if (agg[emoji]) return;
          const add = document.createElement("button");
          add.type = "button";
          add.className = "reaction-add";
          add.textContent = emoji;
          add.title = "React";
          add.onclick = (ev) => {
            ev.stopPropagation();
            toggleReaction(messageId, emoji, false, el);
          };
          el.appendChild(add);
        });
      }


      async function toggleReaction(messageId, emoji, mine, el) {
        if (!requireAuthForWrite()) return;
        try {
          const res = await api(apiWritePath(`/messages/${messageId}/reactions`), {
            method: mine ? "DELETE" : "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ emoji }),
          });
          if (!res.ok) setStatus(`HTTP ${res.status}`, "err");
        } catch (e) {
          setStatus(String(e), "err");
        }
        await loadReactions(messageId, el);
      }


      async function loadPins(threadId) {
        pinnedIds = new Set();
        if (!threadId) return;
        const url = token()
          ? `${base()}/threads/${threadId}/pins`
          : uiReadPath(`/threads/${threadId}/pins`);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (res.ok) {
            const pins = await res.json();
            pins.forEach((p) => pinnedIds.add(p.message_id));
          }
        } catch (e) {
          /* leave pins empty on error */
        }
      }


      async function togglePin(threadId, messageId, pinned) {
        if (!requireAuthForWrite()) return;
        try {
          const res = await api(apiWritePath(`/threads/${threadId}/pins`), {
            method: pinned ? "DELETE" : "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ message_id: messageId }),
          });
          if (!res.ok) {
            setStatus(`HTTP ${res.status}`, "err");
            return;
          }
        } catch (e) {
          setStatus(String(e), "err");
          return;
        }
        await loadMessages();
      }

      let artifactObjectUrls = [];

export { artifactObjectUrls, flashLiveIndicator, liveFrameTargetsOpenThread, liveRefreshTimer, loadMessages, loadPins, loadReactions, pinnedIds, renderMessages, renderSlashResult, scheduleLiveRefresh, togglePin, toggleReaction };
