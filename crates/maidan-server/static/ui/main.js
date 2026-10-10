// @ts-check
import { api, apiReadPath, apiWritePath, base, headers, persist, requireAuthForWrite, requireBearer, token, uiReadPath, wid, writeApi } from "./api.js";
import { attachToSelectedThread, escapeHtml, uploadArtifact } from "./artifacts.js";
import { loadChannels, loadThreads, refreshTeamSoon, selectThread, selectedChannelId, selectedThreadId } from "./board.js";
import { clearDmPickers, initDmPickers, loadDms, loadGroupDms, openDm, openGroupDm, sendDmMessage, sendGroupDmMessage } from "./dm.js";
import { humanError, responseError, setOut, showError, textError, toggleLiveFeed, unreachable } from "./feedback.js";
import { openConnect, openPalette, openTool } from "./palette.js";
import { authorId, loadMembers } from "./people.js";
import { registerBrowserPush } from "./push.js";
import { connectWs, disconnectWs, reconnectNowIfWanted, setPresence } from "./realtime.js";
import { initWorkspaceSwitcher, loadServerAuth, oidcLoginPath, saveWorkspaceName, sessionMemberId, showConnection, showSecretOnce, start } from "./session.js";
import { WORKER_PRESET, baseInput, onMac } from "./state.js";
import { loadMessages } from "./thread.js";
import { capsExceedingGrant, clearPrefsEmail, currentTokenId, followTarget, glassArtifact, glassEventsByKind, glassPeers, glassThread, initTablist, loadApprovals, loadAttenuationCeiling, openConfirmation, loadDeliveries, loadGlass, loadGlobalAudit, loadMessageEdits, loadNotifications, loadPeers, loadPrefs, loadSession, loadSlashCommands, loadWaiting, loadWork, loadWorkDepth, loadWorkThreads, markAllNotificationsRead, myCapabilities, parseCaps, pollReindex, registerSlashCommand, rotateToken, setPrefsDeliveryMode, setPrefsEmail, setPrefsMute, startReindex } from "./tools.js";


      setInterval(refreshTeamSoon, 30000);


      document.querySelectorAll(".tabs button").forEach((btn) => {
        btn.onclick = () => {
          document.querySelectorAll(".tabs button").forEach((b) => {
            b.setAttribute("aria-selected", "false");
            b.tabIndex = -1;
          });
          btn.setAttribute("aria-selected", "true");
          btn.tabIndex = 0;
          document.querySelectorAll(".panel").forEach((p) => p.classList.remove("active"));
          document.getElementById("panel-" + btn.dataset.tab).classList.add("active");
          if (btn.dataset.tab === "work") loadWork();
          if (btn.dataset.tab === "prefs") loadPrefs();
          if (btn.dataset.tab === "glass") loadGlass();
          if (btn.dataset.tab === "notifications") loadNotifications();
          if (btn.dataset.tab === "approvals") loadApprovals().then(() => openConfirmation());
          if (btn.dataset.tab === "session") loadSession();
          if (btn.dataset.tab === "tokens") loadAttenuationCeiling();
        };
      });

      initTablist();

      document.querySelectorAll("[data-open-connect]").forEach((b) => (b.onclick = openConnect));

      document.getElementById("connect-close").onclick = () => document.getElementById("connect-dialog").close();

      document.getElementById("cx-mint").onclick = () => {
        document.getElementById("connect-dialog").close();
        openTool("tokens");
      };

      document.getElementById("cx-create-agent").onclick = async () => {
        const button = document.getElementById("cx-create-agent");
        const status = document.getElementById("cx-status");
        const secretBox = document.getElementById("cx-secret");
        if (!requireBearer()) {
          status.textContent = "Sign in with an admin token first (maidan init prints one).";
          return;
        }
        if (!wid()) {
          status.textContent = "Set a workspace first.";
          return;
        }
        const handle = document.getElementById("cx-handle").value.trim();
        const display = document.getElementById("cx-name").value.trim();
        if (!handle) {
          status.textContent = "A handle is required. It is the name shown for that member on the board.";
          return;
        }
        // One submission at a time. The one-time secret stays until a new
        // token is actually minted, so a failed retry does not erase it.
        if (button.disabled) return;
        button.disabled = true;
        persist();
        status.textContent = "Creating the member…";
        // Once the member exists a retry cannot create it again (the handle
        // is taken), so every failure after that point names it and puts its
        // id in Tokens, where a token can be minted for it.
        let member = null;
        const memberCreated = (what) => {
          document.getElementById("token-member").value = member.id;
          return `Member ${handle} was created, but ${what} Mint its token in Tokens; the member is filled in there.`;
        };
        try {
          let res;
          let minted = null;
          // One call creates the agent and its worker token together
          // (POST /workspaces/{wid}/agents, token:admin). It is on every
          // server image, so a hosted instance needs no bootstrap route.
          try {
            res = await writeApi(null, `${base()}/workspaces/${wid()}/agents`, {
              method: "POST",
              headers: headers(true),
              body: JSON.stringify({ handle, display_name: display || null }),
            });
          } catch (e) {
            // The request may have reached the server: the agent may exist.
            status.textContent = `The server may have created ${handle} before the reply was lost. Check the member list before trying again. ${unreachable(e)}`;
            return;
          }
          if (res.ok) {
            const created = await res.json();
            member = created.member;
            minted = created.token;
          } else if (res.status !== 404) {
            // A refusal: nothing was created, member and token commit together.
            status.textContent = await responseError(res, "Could not connect the agent");
            return;
          } else {
            // An older server without that route: create the member, then
            // mint its token, the two calls this page made before.
            try {
              // The outer guard owns the button for the whole create+mint action:
              // passing null keeps writeApi from re-enabling it between the
              // two requests, where another click could start an overlap.
              res = await writeApi(null, `${base()}/workspaces/${wid()}/members`, {
                method: "POST",
                headers: headers(true),
                body: JSON.stringify({ handle, display_name: display || null, kind: "agent" }),
              });
            } catch (e) {
              status.textContent = unreachable(e);
              return;
            }
            if (!res.ok) {
              status.textContent = await responseError(res, "Could not create the member");
              return;
            }
            member = await res.json();
            status.textContent = "Minting a worker token…";
            try {
              res = await writeApi(null, `${base()}/workspaces/${wid()}/members/${member.id}/tokens`, {
                method: "POST",
                headers: headers(true),
                body: JSON.stringify({ label: handle, capability_set: WORKER_PRESET, capabilities: [] }),
              });
            } catch (e) {
              // The request may have reached the server before the reply was
              // lost, so a token may exist that this page never saw: the
              // outcome is unknown, not a confirmed failure.
              status.textContent = memberCreated(`the token request may have reached the server, so a token may exist that this page never saw. ${unreachable(e)}`);
              return;
            }
            if (!res.ok) {
              // mint_api_token can commit the token before the quota listing
              // fails, so a 5xx leaves the outcome unknown: a token may exist
              // that this page never saw. A 4xx is a refusal before anything
              // was created.
              const what =
                res.status >= 500
                  ? "the server failed while minting its token, so a token may exist that this page never saw."
                  : `${await responseError(res, "the server refused to mint its token")}.`;
              status.textContent = memberCreated(what);
              return;
            }
            minted = await res.json();
          }
          secretBox.hidden = false;
          secretBox.replaceChildren();
          document.getElementById("token-member").value = member.id;
          const caps = (minted.capabilities || []).join(", ");
          const lead = document.createElement("span");
          lead.textContent = `Token for ${display || handle} (shown once). It can claim, post, and transition. Paste it where the snippet says REPLACE_WITH_MAIDAN_TOKEN. `;
          const code = document.createElement("code");
          code.id = "cx-secret-value";
          code.textContent = minted.secret;
          const copy = document.createElement("button");
          copy.type = "button";
          copy.textContent = "Copy token";
          copy.onclick = async () => {
            try {
              await navigator.clipboard.writeText(minted.secret);
              status.textContent = "Token copied. The snippets still show a placeholder.";
            } catch (_e) {
              status.textContent = "Copy was blocked by the browser; select the token instead.";
            }
          };
          secretBox.append(lead, code, document.createTextNode(" "), copy);
          if (caps) secretBox.append(document.createTextNode(" Capabilities: " + caps + "."));
          status.textContent = `Member ${handle} created. The worker preset is ${WORKER_PRESET}.`;
          loadMembers();
        } catch (_e) {
          // A reply that could not be read. After a mint the server holds a
          // token this page never saw: it cannot be shown again, so say to
          // mint another rather than leave "Minting…" on screen.
          status.textContent = member
            ? memberCreated("the reply with its token could not be read, so that token cannot be shown.")
            : "The member reply could not be read. Check Tokens or the member list before trying again: the member may exist.";
        } finally {
          button.disabled = false;
        }
      };

      document.querySelectorAll("#connect-dialog [data-copy]").forEach((b) => {
        b.onclick = async () => {
          const text = document.getElementById(b.dataset.copy).textContent;
          const status = document.getElementById("cx-status");
          try {
            await navigator.clipboard.writeText(text);
            status.textContent = "Copied.";
          } catch (_e) {
            status.textContent = "Copy was blocked by the browser; select the text instead.";
          }
        };
      });

      document.querySelectorAll(".kbd.mod-k").forEach((k) => (k.textContent = onMac ? "⌘K" : "Ctrl K"));

      document.getElementById("palette-open").onclick = openPalette;

      document.addEventListener("keydown", (e) => {
        const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement && document.activeElement.tagName);
        if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
          e.preventDefault();
          openPalette();
        } else if (e.key === "/" && !typing && !document.querySelector("dialog[open]")) {
          e.preventDefault();
          openPalette();
        }
      });


      // Approval gates are a human-in-the-loop queue, not a manual-refresh
      // report. Keep the visible panel fresh without polling a hidden tab.
      setInterval(() => {
        const panel = document.getElementById("panel-approvals");
        if (panel.classList.contains("active") && document.visibilityState === "visible") {
          loadApprovals(false);
        }
      }, 5000);

      window.addEventListener("online", reconnectNowIfWanted);

      document.addEventListener("visibilitychange", () => {
        if (document.visibilityState === "visible") reconnectNowIfWanted();
      });

      baseInput.addEventListener("change", loadServerAuth);

      initWorkspaceSwitcher();

      document.getElementById("login").onclick = () => {
        if (!oidcLoginPath) return;
        if (!wid()) return showError("Enter the workspace ID first.");
        persist();
        window.location.href =
          `${base()}${oidcLoginPath}?workspace_id=${encodeURIComponent(wid())}&return_to=/ui/`;
      };

      document.getElementById("mint").onclick = async () => {
        const res = await writeApi("mint", `${base()}/auth/session/mint`, {
          method: "POST",
          credentials: "include",
        });
        const body = await res.text();
        if (!res.ok) return setOut(body);
        const parsed = JSON.parse(body);
        document.getElementById("token").value = parsed.secret;
        persist();
        showSecretOnce("Admin token (once)", parsed.secret);
        showError("Admin token minted", "success");
      };

      document.getElementById("rotate-own-token").onclick = () => {
        if (currentTokenId) rotateToken(currentTokenId, "rotate-own-token");
      };

      document.getElementById("rotate-token").onclick = () => {
        if (!requireBearer()) return;
        const id = document.getElementById("token-revoke-id").value.trim();
        if (!id) return showError("Token ID required");
        rotateToken(id, "rotate-token");
      };

      document.getElementById("copy-secret").onclick = () => {
        const s = document.getElementById("mint-secret").textContent;
        if (s) navigator.clipboard.writeText(s);
      };

      document.getElementById("refresh-channels").onclick = loadChannels;

      document.getElementById("ws-connect").onclick = connectWs;

      document.getElementById("ws-disconnect").onclick = disconnectWs;

      document.getElementById("presence-online").onclick = () => setPresence("online");

      document.getElementById("presence-away").onclick = () => setPresence("away");

      document.getElementById("work-refresh").onclick = () => loadWork();

      document.getElementById("waiting-refresh").onclick = () => loadWaiting();

      document.getElementById("work-channel").onchange = () => {
        loadWorkDepth();
        loadWorkThreads();
      };

      document.getElementById("prefs-refresh").onclick = () => loadPrefs();

      document.getElementById("prefs-mode-immediate").onclick = () =>
        setPrefsDeliveryMode("immediate");

      document.getElementById("prefs-mode-digest").onclick = () =>
        setPrefsDeliveryMode("digest");

      document.getElementById("prefs-email-set").onclick = () => setPrefsEmail();

      document.getElementById("prefs-email-clear").onclick = () => clearPrefsEmail();

      document.getElementById("prefs-mute").onclick = () => setPrefsMute(true);

      document.getElementById("prefs-unmute").onclick = () => setPrefsMute(false);

      document.getElementById("prefs-follow-channel-btn").onclick = () =>
        followTarget("channel-follows");

      document.getElementById("prefs-follow-thread-btn").onclick = () =>
        followTarget("thread-follows");

      document.getElementById("glass-kind-btn").onclick = () => glassEventsByKind();

      document.getElementById("glass-thread-btn").onclick = () => glassThread();

      document.getElementById("glass-sha-btn").onclick = () => glassArtifact();

      document.getElementById("glass-peers-btn").onclick = () => glassPeers();

      document.getElementById("notif-refresh").onclick = () => loadNotifications();

      document.getElementById("approvals-refresh").onclick = () => loadApprovals();


      document.getElementById("session-refresh").onclick = () => loadSession();

      document.getElementById("notif-read-all").onclick = () =>
        markAllNotificationsRead();

      document.getElementById("notif-unread-only").onchange = () =>
        loadNotifications();

      document.getElementById("ws-clear").onclick = () => {
        document.getElementById("live-feed").textContent = "";
      };

      document.getElementById("workspace").addEventListener("change", () => {
        persist();
        clearDmPickers();
        loadChannels();
        disconnectWs();
      });


      document.getElementById("load-events").onclick = async () => {
        persist();
        const t = token();
        const url = t
          ? `${base()}/workspaces/${wid()}/events?after_id=0&limit=50`
          : `${base()}/ui/api/workspaces/${wid()}/events?after_id=0&limit=50`;
        const res = await api(url, { headers: headers(), credentials: "include" });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not load events"));
          return setOut(body);
        }
        setOut(JSON.parse(body));
      };

      document.getElementById("run-search").onclick = async () => {
        persist();
        if (!wid()) return showError("Workspace ID required");
        const q = document.getElementById("search-q").value.trim();
        const mode = document.getElementById("search-mode").value;
        const params = new URLSearchParams({ q, mode, limit: "20" });
        const ch =
          document.getElementById("search-channel").value.trim() ||
          selectedChannelId ||
          "";
        if (ch) params.set("channel", ch);
        const author = document.getElementById("search-author").value.trim();
        if (author) params.set("author", author);
        const kind = document.getElementById("search-kind").value;
        if (kind) params.set("kind", kind);
        const url = token()
          ? `${base()}/workspaces/${wid()}/search?${params}`
          : `${uiReadPath(`/workspaces/${wid()}/search`)}?${params}`;
        const box = document.getElementById("search-results");
        let res;
        let body;
        try {
          res = await api(url, { headers: headers(), credentials: "include" });
          body = await res.text();
        } catch (e) {
          box.textContent = unreachable(e);
          showError(unreachable(e));
          return;
        }
        if (!res.ok) {
          let detail = "";
          try {
            const problem = JSON.parse(body);
            detail = problem.detail || problem.title || problem.error || problem.message || "";
          } catch (_e) {
            detail = body;
          }
          detail = String(detail).replace(/\s+/g, " ").trim().slice(0, 500);
          const said = humanError(res.status, detail);
          box.textContent = detail ? `${said} — ${detail} (HTTP ${res.status})` : `${said} (HTTP ${res.status})`;
          showError(said);
          return;
        }
        let hits;
        try {
          hits = JSON.parse(body);
        } catch (e) {
          box.textContent = unreachable(e);
          showError("Could not read the search results");
          return;
        }
        box.innerHTML = "";
        if (!hits.length) {
          box.textContent = "No hits";
          setOut(hits);
          return;
        }
        hits.forEach((h) => {
          const div = document.createElement("div");
          div.className = "hit";
          div.innerHTML = `<strong>${escapeHtml(h.message_id || h.id || "?")}</strong> ${escapeHtml(h.body || h.snippet || "")}`;
          box.appendChild(div);
        });
        setOut(hits);
      };

      document.getElementById("create-channel").onclick = async () => {
        if (!requireAuthForWrite()) return;
        if (!wid()) return showError("Workspace ID required");
        const name = document.getElementById("new-channel-name").value.trim();
        if (!name) return showError("Channel name required");
        persist();
        const res = await writeApi("create-channel", apiWritePath(`/workspaces/${wid()}/channels`), {
          method: "POST",
          headers: headers(true),
          credentials: "include",
          body: JSON.stringify({ name, private: false }),
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not create the channel"));
          return setOut(body);
        }
        document.getElementById("new-channel-name").value = "";
        showError("Channel created", "success");
        setOut(JSON.parse(body));
        await loadChannels();
      };

      document.getElementById("create-thread").onclick = async () => {
        if (!requireAuthForWrite()) return;
        if (!selectedChannelId) return showError("Select a channel first");
        const title = document.getElementById("new-thread-title").value.trim();
        if (!title) return showError("Thread title required");
        persist();
        const res = await writeApi("create-thread",
          apiWritePath(`/channels/${selectedChannelId}/threads`),
          {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ title }),
          }
        );
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not create the thread"));
          return setOut(body);
        }
        const parsed = JSON.parse(body);
        document.getElementById("new-thread-title").value = "";
        showError("Thread created", "success");
        setOut(parsed);
        await loadThreads();
        selectThread(parsed.id, parsed.title || parsed.id);
      };

      document.getElementById("post-message").onclick = async () => {
        if (!requireAuthForWrite()) return;
        if (!selectedThreadId) return showError("Select a thread first");
        const text = document.getElementById("compose-body").value.trim();
        if (!text) return showError("Message body required");
        persist();
        let res;
        try {
          res = await writeApi("post-message",
            apiWritePath(`/threads/${selectedThreadId}/messages`),
            {
              method: "POST",
              headers: headers(true),
              credentials: "include",
              body: JSON.stringify({ body: text }),
            }
          );
        } catch (e) {
          // The draft stays. A dead server used to reject the promise and
          // leave the composer looking as if nothing had been tried.
          showError(unreachable(e));
          return;
        }
        if (!res.ok) {
          showError(await responseError(res, "Could not post"));
          return;
        }
        const body = await res.text();
        document.getElementById("compose-body").value = "";
        showError("Posted", "success");
        setOut(JSON.parse(body));
        await loadMessages();
      };

      document.getElementById("reload-messages").onclick = loadMessages;

      initDmPickers();

      document.getElementById("gdm-open").onclick = openGroupDm;

      document.getElementById("gdm-refresh").onclick = loadGroupDms;

      document.getElementById("gdm-send").onclick = sendGroupDmMessage;

      document.getElementById("dm-open").onclick = openDm;

      document.getElementById("dm-refresh").onclick = loadDms;

      document.getElementById("dm-send").onclick = sendDmMessage;

      document.getElementById("op-deliv-refresh").onclick = loadDeliveries;

      document.getElementById("op-audit-load").onclick = loadGlobalAudit;

      document.getElementById("op-reindex-workspace").onclick = () => startReindex(false);

      document.getElementById("op-reindex-global").onclick = () => startReindex(true);

      document.getElementById("op-reindex-poll").onclick = pollReindex;

      document.getElementById("slash-register").onclick = registerSlashCommand;

      document.getElementById("slash-refresh").onclick = loadSlashCommands;

      document.getElementById("edit-message").onclick = async () => {
        if (!requireAuthForWrite()) return;
        const id = document.getElementById("edit-message-id").value.trim();
        if (!id) return showError("Message ID required");
        const text = document.getElementById("edit-message-body").value.trim();
        persist();
        const res = await writeApi("edit-message", apiWritePath(`/messages/${id}`), {
          method: "PATCH",
          headers: headers(true),
          credentials: "include",
          body: JSON.stringify({ body: text }),
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not edit the message"));
          return setOut(body);
        }
        showError("Edited", "success");
        const parsed = JSON.parse(body);
        setOut(parsed);
        await loadMessages();
        await loadMessageEdits(id);
      };

      document.getElementById("load-edit-history").onclick = () => {
        const id = document.getElementById("edit-message-id").value.trim();
        loadMessageEdits(id);
      };

      document.getElementById("upload-artifact").onclick = async () => {
        const btn = document.getElementById("upload-artifact");
        // Hold the button through the optional attach: writeApi would
        // otherwise re-enable it after the upload while the attach is still
        // in flight, and another click could upload and attach twice.
        if (btn.disabled) return;
        btn.disabled = true;
        try {
          if (!requireAuthForWrite()) return;
          const fileInput = document.getElementById("artifact-file");
          if (!fileInput.files || !fileInput.files[0]) return showError("Choose a file");
          const kind = document.getElementById("artifact-kind").value;
          persist();
          const artifact = await uploadArtifact(fileInput.files[0], kind, null);
          if (!artifact) return;
          showError("Artifact uploaded", "success");
          setOut(artifact);
          const attach = document.getElementById("attach-artifact-next");
          if (attach && attach.checked) await attachToSelectedThread(artifact);
        } finally {
          btn.disabled = false;
        }
      };

      // Paste a file (a screenshot, say) into the composer and it becomes an
      // artifact attached to the selected thread. Pasting text is untouched.
      document.getElementById("compose-body").addEventListener("paste", async (event) => {
        const files = Array.from((event.clipboardData && event.clipboardData.files) || []);
        if (!files.length) return;
        event.preventDefault();
        if (!requireAuthForWrite()) return;
        if (!selectedThreadId) {
          showError("Select a thread before pasting a file");
          return;
        }
        persist();
        for (const file of files) {
          const kind = file.type.startsWith("image/") ? "screenshot" : "attachment";
          const artifact = await uploadArtifact(file, kind);
          if (!artifact || !(await attachToSelectedThread(artifact))) return;
        }
      });

      document.getElementById("load-thread").onclick = async () => {
        persist();
        const id = document.getElementById("thread-id").value.trim();
        const res = await api(apiReadPath(`/threads/${id}`), {
          headers: headers(),
          credentials: "include",
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not load the thread"));
          return setOut(body);
        }
        setOut(JSON.parse(body));
      };

      document.getElementById("transition-thread").onclick = async () => {
        if (!requireAuthForWrite()) return;
        persist();
        const id = document.getElementById("thread-id").value.trim();
        const action = document.getElementById("fsm-action").value;
        const res = await writeApi("transition-thread", apiWritePath(`/threads/${id}`), {
          method: "POST",
          headers: headers(true),
          credentials: "include",
          body: JSON.stringify({ action }),
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, `Could not apply ${action} to the thread`));
          return setOut(body);
        }
        showError(`Applied ${action} to the thread`, "success");
        setOut(JSON.parse(body));
      };

      document.getElementById("load-messages").onclick = async () => {
        persist();
        const id = document.getElementById("thread-id").value.trim();
        const res = await api(`${base()}/threads/${id}/messages?limit=50`, {
          headers: headers(),
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not load the messages"));
          return setOut(body);
        }
        setOut(JSON.parse(body));
      };


      document.getElementById("load-audit").onclick = async () => {
        if (!wid()) return showError("Workspace ID required");
        persist();
        const limit = document.getElementById("audit-limit").value || "50";
        const url = `${uiReadPath(`/workspaces/${wid()}/audit`)}?limit=${encodeURIComponent(limit)}`;
        const res = await api(url, { headers: headers(), credentials: "include" });
        const body = await res.text();
        const box = document.getElementById("audit-list");
        if (!res.ok) {
          box.textContent = body;
          showError(textError(res.status, body, "Could not load the audit log"));
          return;
        }
        const rows = JSON.parse(body);
        box.innerHTML = "";
        if (!rows.length) {
          box.textContent = "No audit rows";
          return;
        }
        rows.forEach((r) => {
          const div = document.createElement("div");
          div.className = "row";
          div.textContent = `${r.occurred_at} · ${r.action} · actor=${r.actor_id || "—"}`;
          box.appendChild(div);
        });
        setOut(rows);
      };


      document.getElementById("purge-workspace").onclick = async () => {
        if (!requireBearer()) return;
        const w = wid();
        if (!w) return showError("Workspace ID required");
        if (document.getElementById("purge-confirm").value.trim() !== w) {
          return showError("Confirmation must match workspace ID exactly");
        }
        if (!document.getElementById("purge-understand").checked) {
          return showError("Check the confirmation box");
        }
        persist();
        const res = await writeApi("purge-workspace", `${base()}/workspaces/${w}/purge`, {
          method: "POST",
          headers: headers(true),
          body: "{}",
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not purge the workspace"));
          return setOut(body);
        }
        showError("Workspace purged", "success");
        setOut(JSON.parse(body));
        document.getElementById("load-audit").click();
      };


      document.getElementById("refresh-peers").onclick = loadPeers;

      document.getElementById("create-peer").onclick = async () => {
        if (!requireBearer()) return;
        if (!wid()) return showError("Workspace ID required");
        const name = document.getElementById("peer-name").value.trim();
        const baseUrl = document.getElementById("peer-base-url").value.trim();
        if (!name || !baseUrl) return showError("Name and base URL required");
        persist();
        const res = await writeApi("create-peer", `${base()}/workspaces/${wid()}/peers`, {
          method: "POST",
          headers: headers(true),
          body: JSON.stringify({ name, base_url: baseUrl }),
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not create the peer"));
          return setOut(body);
        }
        const parsed = JSON.parse(body);
        showError("Peer created — copy secret now", "success");
        setOut(parsed);
        await loadPeers();
      };

      document.getElementById("delete-peer").onclick = async () => {
        if (!requireBearer()) return;
        const pid = document.getElementById("peer-delete-id").value.trim();
        if (!pid || !wid()) return showError("Peer ID and workspace required");
        persist();
        const res = await writeApi("delete-peer", `${base()}/workspaces/${wid()}/peers/${pid}`, {
          method: "DELETE",
          headers: headers(),
        });
        if (!res.ok) {
          const body = await res.text();
          showError(textError(res.status, body, "Could not delete the peer"));
          return setOut(body);
        }
        showError("Peer deleted", "success");
        setOut({ deleted: pid });
        await loadPeers();
      };


      document.getElementById("mint-member-token").onclick = async () => {
        const btn = document.getElementById("mint-member-token");
        // One mint at a time, guarded before the attenuation preflight: two
        // rapid clicks could otherwise both pass the await below and mint
        // with different idempotency keys.
        if (btn.disabled) return;
        btn.disabled = true;
        try {
          if (!requireBearer()) return;
          persist();
          const mid = document.getElementById("token-member").value.trim();
          const label = document.getElementById("token-label").value.trim();
          const caps = parseCaps(document.getElementById("token-caps").value);
          // Attenuation pre-flight: a minted token cannot exceed the caller's grant
          // (the server enforces this too, via validate_subset — this flags it early).
          if (!myCapabilities) await loadAttenuationCeiling();
          const excess = capsExceedingGrant(caps);
          const warn = document.getElementById("attenuation-warning");
          if (excess.length) {
            warn.textContent =
              `Cannot widen your grant — these exceed your ceiling and will be rejected: ${excess.join(", ")}`;
            warn.hidden = false;
            showError("Attenuation: request exceeds your grant", "warning");
            return;
          }
          warn.hidden = true;
          const res = await writeApi(null, `${base()}/workspaces/${wid()}/members/${mid}/tokens`, {
            method: "POST",
            headers: headers(true),
            body: JSON.stringify({ label: label || null, capabilities: caps }),
          });
          const body = await res.text();
          if (!res.ok) {
            showError(textError(res.status, body, "Could not mint the token"));
            return setOut(body);
          }
          const parsed = JSON.parse(body);
          document.getElementById("token").value = parsed.secret;
          document.getElementById("token-revoke-id").value = parsed.id;
          persist();
          showError("Token minted", "success");
          setOut(parsed);
        } finally {
          btn.disabled = false;
        }
      };

      document.getElementById("list-member-tokens").onclick = async () => {
        if (!wid()) return showError("Workspace ID required");
        const mid =
          document.getElementById("token-member").value.trim() || sessionMemberId;
        if (!mid) return showError("Member ID required");
        persist();
        const path = token()
          ? `${base()}/workspaces/${wid()}/members/${mid}/tokens`
          : `${uiReadPath(`/workspaces/${wid()}/members/${mid}/tokens`)}`;
        const res = await api(path, { headers: headers(), credentials: "include" });
        const body = await res.text();
        const box = document.getElementById("token-list");
        if (!res.ok) {
          box.textContent = body;
          return;
        }
        const rows = JSON.parse(body);
        box.innerHTML = "";
        if (!rows.length) {
          box.textContent = "No tokens for this member";
          return;
        }
        rows.forEach((t) => {
          const li = document.createElement("div");
          const revoked = t.revoked_at ? " (revoked)" : "";
          li.textContent = `${t.id} · ${(t.label || "—")}${revoked} · ${t.capabilities.join(", ")}`;
          li.classList.add("token-row");
          li.onclick = () => {
            document.getElementById("token-revoke-id").value = t.id;
          };
          box.appendChild(li);
        });
      };

      document.getElementById("list-app-installations").onclick = async () => {
        if (!wid()) return showError("Workspace ID required");
        persist();
        const path = uiReadPath(`/workspaces/${wid()}/app-installations`);
        const res = await api(path, { headers: headers(), credentials: "include" });
        const body = await res.text();
        const box = document.getElementById("app-install-list");
        if (!res.ok) {
          box.textContent = body;
          return;
        }
        const rows = JSON.parse(body);
        box.innerHTML = "";
        if (!rows.length) {
          box.textContent = "No app installations";
          return;
        }
        rows.forEach((row) => {
          const div = document.createElement("div");
          const revoked = row.revoked_at ? " (revoked)" : "";
          div.textContent = `${row.app_id} · install ${row.id}${revoked}`;
          box.appendChild(div);
        });
      };

      document.getElementById("revoke-token").onclick = async () => {
        if (!requireBearer()) return;
        const id = document.getElementById("token-revoke-id").value.trim();
        if (!id) return showError("Token ID required");
        persist();
        const res = await writeApi("revoke-token", `${base()}/tokens/${id}`, {
          method: "DELETE",
          headers: headers(),
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not revoke the token"));
          return setOut(body);
        }
        showError("Token revoked", "success");
        setOut(body ? JSON.parse(body) : { revoked: id });
      };

      document.getElementById("live-toggle").onclick = () => toggleLiveFeed();

      document.getElementById("conn-edit").onclick = () => showConnection(true);

      document.getElementById("workspace-name-save").onclick = () => saveWorkspaceName();

      start().then(() => {
        registerBrowserPush(authorId());
        // A confirmation link lands here, #confirm-approval=<gate>.<token>,
        // once the session has named its workspace.
        if (typeof location !== "undefined" && location.hash.startsWith("#confirm-approval=")) openTool("approvals");
      });

      // The link is a fragment, so opening it in a console that is already
      // loaded does not run start() again.
      if (typeof window !== "undefined") {
        window.addEventListener("hashchange", () => {
          if (typeof location !== "undefined" && location.hash.startsWith("#confirm-approval=")) openTool("approvals");
        });
      }

    
