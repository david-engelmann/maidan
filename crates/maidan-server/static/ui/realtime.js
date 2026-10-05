// @ts-check
import { pastedToken, persist, token, wid, wsUrl } from "./api.js";
import { loadThreads, markSeen, markSeenFromFrame, noteRefusalFromFrame, refreshTeamSoon, scheduleBoardRefresh, selectedChannelId, selectedThreadId } from "./board.js";
import { appendLive, setStatus, setWsStatus, showError } from "./feedback.js";
import { loadNeedsYou } from "./needs.js";
import { authorId, memberName } from "./people.js";
import { sessionMemberId } from "./session.js";
import { LIVE_POLL_MS, THREAD_BOARD_KINDS, wsResumeKey } from "./state.js";
import { liveFrameTargetsOpenThread, scheduleLiveRefresh } from "./thread.js";
import { loadApprovals } from "./tools.js";


      let wsSocket = null;

      let wsAfterId = 0;

      let wsResumeToken = localStorage.getItem(wsResumeKey) || null;

      let wsUserDisconnect = false;


      function buildWsFilter() {
        const preset = document.getElementById("ws-preset").value;
        const filter = { workspace_id: wid() };
        if (preset === "channel" && selectedChannelId) {
          filter.channel_id = selectedChannelId;
        } else if (preset === "thread" && selectedThreadId) {
          filter.thread_id = selectedThreadId;
        } else if (preset === "messages") {
          filter.kinds = ["message_posted", "mention_recorded"];
        }
        return filter;
      }


      function disconnectWs() {
        wsUserDisconnect = true;
        wsWanted = false;
        clearTimeout(wsRetryTimer);
        setLiveFallback(false);
        if (wsSocket) {
          const old = wsSocket;
          wsSocket = null;
          old.close();
        }
        document.getElementById("ws-connect").hidden = false;
        document.getElementById("ws-disconnect").hidden = true;
        setWsStatus("disconnected");
      }


      function renderPresence(frame) {
        const list = document.getElementById("presence-list");
        list.innerHTML = "";
        const members = (frame && frame.members) || [];
        if (!members.length) {
          list.innerHTML = "<li>No one online</li>";
          return;
        }
        const me = authorId();
        members.forEach((m) => {
          const li = document.createElement("li");
          const mine = m.member_id === me ? " (you)" : "";
          li.textContent = `${memberName(m.member_id)} · ${m.status}${mine}`;
          li.title = m.member_id;
          list.appendChild(li);
        });
      }


      function setPresence(status) {
        if (!wsSocket || wsSocket.readyState !== WebSocket.OPEN) {
          return setStatus("Press Connect WS in the Live toolbar first", "err");
        }
        wsSocket.send(JSON.stringify({ type: "presence", status }));
        setStatus(`Presence → ${status}`, "ok");
      }


      // Reconnect after a drop with backoff (1.5 s doubling to 30 s), and keep
      // the board fresh by polling while the socket is down. A socket replaced
      // by a newer one is ignored: its late close must not touch the new one.
      let wsWanted = false;

      let wsRetries = 0;

      let wsRetryTimer = null;

      let liveFallbackTimer = null;

      function setLiveFallback(on) {
        if (on && !liveFallbackTimer) {
          liveFallbackTimer = setInterval(() => {
            if (selectedChannelId) loadThreads(true);
            loadNeedsYou();
          }, LIVE_POLL_MS);
        } else if (!on && liveFallbackTimer) {
          clearInterval(liveFallbackTimer);
          liveFallbackTimer = null;
        }
      }

      function scheduleReconnect() {
        clearTimeout(wsRetryTimer);
        const delay = Math.min(30000, 1500 * 2 ** wsRetries) * (0.8 + Math.random() * 0.4);
        wsRetries += 1;
        setWsStatus(`reconnecting in ${Math.round(delay / 1000)} s · board refreshes every ${LIVE_POLL_MS / 1000} s`, "error");
        setLiveFallback(true);
        wsRetryTimer = setTimeout(() => connectWs(), delay);
      }

      function reconnectNowIfWanted() {
        if (wsWanted && !wsSocket && !wsUserDisconnect) {
          clearTimeout(wsRetryTimer);
          connectWs();
        }
      }

      function connectWs() {
        clearTimeout(wsRetryTimer);
        if (wsSocket) {
          const old = wsSocket;
          wsSocket = null;
          old.close();
        }
        wsUserDisconnect = false;
        if (!wid()) return showError("Workspace ID required");
        const preset = document.getElementById("ws-preset").value;
        if (preset === "channel" && !selectedChannelId) {
          return showError("Select a channel for the channel preset");
        }
        if (preset === "thread" && !selectedThreadId) {
          return showError("Select a thread for the thread preset");
        }
        if (!token() && !sessionMemberId) {
          return showError("Sign in first: live updates need a signed-in session or a token.");
        }
        persist();
        // Wanted from the first attempt, not from the ack: a drop before the
        // ack (a restart, a network blip) is retried like any other. A policy
        // refusal (1008) still stops, and Disconnect still clears this.
        wsWanted = true;
        const frame = { filter: buildWsFilter() };
        const t = pastedToken();
        if (t) frame.token = t;
        if (wsResumeToken) {
          frame.resume_token = wsResumeToken;
        } else {
          frame.after_id = wsAfterId;
        }
        // Presence and typing go to the member the socket authenticates as:
        // the bearer's member when a token is set, else the session's. That is
        // also who the header calls "you".
        const presenceId = authorId();
        if (presenceId) frame.member_id = presenceId;

        if (!wsRetries) setWsStatus("connecting…");
        const sock = new WebSocket(wsUrl());
        wsSocket = sock;
        sock.onopen = () => {
          if (sock === wsSocket) sock.send(JSON.stringify(frame));
        };
        sock.onmessage = (ev) => {
          if (sock !== wsSocket) return;
          let v;
          try {
            v = JSON.parse(ev.data);
          } catch {
            appendLive(ev.data);
            return;
          }
          const t = v.type;
          if (t === "subscribe_ack") {
            const recovered = wsRetries > 0;
            wsRetries = 0;
            setLiveFallback(false);
            setWsStatus("connected", "connected");
            // Back after a drop: reconcile what the replay may not cover.
            if (recovered) {
              scheduleBoardRefresh();
              loadNeedsYou();
            }
            document.getElementById("ws-connect").hidden = true;
            document.getElementById("ws-disconnect").hidden = false;
            if (typeof v.after_id === "number") wsAfterId = v.after_id;
            if (v.resume_token) {
              wsResumeToken = v.resume_token;
              localStorage.setItem(wsResumeKey, wsResumeToken);
            }
            appendLive(`[ack] after_id=${v.after_id}`);
            return;
          }
          if (t === "presence_snapshot") {
            ((v && v.members) || []).forEach((m) => m.status === "online" && markSeen(m.member_id));
            refreshTeamSoon();
            renderPresence(v);
            appendLive(JSON.stringify(v));
            return;
          }
          if (t === "presence" || t === "typing") {
            if (v.status !== "offline") markSeen(v.member_id);
            refreshTeamSoon();
            appendLive(JSON.stringify(v));
            return;
          }
          if (t === "replay_hint" || t === "replay_truncated") {
            appendLive(JSON.stringify(v));
            return;
          }
          if (typeof v.log_id === "number") {
            wsAfterId = Math.max(wsAfterId, v.log_id);
            const kind = v.kind || "?";
            appendLive(`[${v.log_id}] ${kind}`);
            markSeenFromFrame(v);
            refreshTeamSoon();
            if (kind === "approval_requested") {
              if (document.getElementById("panel-approvals").classList.contains("active")) {
                loadApprovals(false);
              }
              loadThreads();
            }
            if (liveFrameTargetsOpenThread(v)) scheduleLiveRefresh();
            if (THREAD_BOARD_KINDS.has(kind)) scheduleBoardRefresh();
            if (kind === "message_posted") noteRefusalFromFrame(v);
            return;
          }
          if (v.kind) {
            appendLive(JSON.stringify(v));
            return;
          }
          appendLive(JSON.stringify(v));
        };
        sock.onerror = () => {
          if (sock === wsSocket && !wsRetries) setWsStatus("error", "error");
        };
        sock.onclose = (ev) => {
          if (sock !== wsSocket) return;
          const wasConnected = document.getElementById("ws-disconnect").hidden === false;
          document.getElementById("ws-connect").hidden = false;
          document.getElementById("ws-disconnect").hidden = true;
          if (wasConnected) setWsStatus("closed");
          // Refused before the ack (bad token, missing event:subscribe): say
          // why instead of sitting on "connecting…".
          else if (!wsUserDisconnect)
            setWsStatus(ev && ev.reason ? `refused: ${ev.reason}` : "could not connect", "error");
          wsSocket = null;
          // A refusal (policy close 1008: bad token, missing capability) will
          // not fix itself, so say why and stop. A drop is retried with
          // backoff for as long as the socket had been wanted.
          const refused = ev && ev.code === 1008;
          if (
            wsWanted &&
            !refused &&
            !wsUserDisconnect &&
            document.getElementById("ws-auto-reconnect").checked
          ) {
            scheduleReconnect();
          } else if (refused) {
            wsWanted = false;
            setLiveFallback(false);
          }
        };
      }

export { buildWsFilter, connectWs, disconnectWs, liveFallbackTimer, reconnectNowIfWanted, renderPresence, scheduleReconnect, setLiveFallback, setPresence, wsAfterId, wsResumeToken, wsRetries, wsRetryTimer, wsSocket, wsUserDisconnect, wsWanted };
