// @ts-check
import { api, apiWritePath, base, headers, persist, requireAuthForWrite, token, uiReadPath, wid } from "./api.js";
import { escapeHtml } from "./artifacts.js";
import { clearLoading, renderState, responseError, setLoading, setOut, setStatus, showError, unreachable } from "./feedback.js";
import { authorId, memberName } from "./people.js";
import { connectWs, disconnectWs, wsSocket } from "./realtime.js";
import { exchangeToken, sessionMemberId, showSecretOnce } from "./session.js";


      let approvalsLoading = false;


      function renderDeliveries(rows) {
        const box = document.getElementById("op-deliv-list");
        box.innerHTML = "";
        if (!rows.length) {
          box.textContent = "No deliveries";
          return;
        }
        rows.forEach((d) => {
          const card = document.createElement("div");
          card.className = "deliv";
          const head = document.createElement("div");
          head.className = "deliv-head";
          const tag = document.createElement("span");
          tag.className = "deliv-kind";
          tag.textContent = d.kind;
          head.appendChild(tag);
          const meta = document.createElement("span");
          meta.textContent = ` #${d.id} · ${d.attempts} attempt${d.attempts === 1 ? "" : "s"}`;
          head.appendChild(meta);
          if (d.quarantined_at) {
            const q = document.createElement("span");
            q.className = "deliv-dlq";
            q.textContent = " DLQ";
            head.appendChild(q);
          } else if (d.delivered_at) {
            const ok = document.createElement("span");
            ok.textContent = " ✓ delivered";
            head.appendChild(ok);
          }
          card.appendChild(head);
          const url = document.createElement("div");
          url.textContent = d.target_url;
          card.appendChild(url);
          if (d.last_error) {
            const err = document.createElement("div");
            err.className = "deliv-err";
            err.textContent = `error: ${d.last_error}`;
            card.appendChild(err);
          }
          const ts = document.createElement("div");
          ts.className = "muted";
          const next = d.next_attempt_at ? ` · next: ${d.next_attempt_at}` : "";
          ts.textContent = `created: ${d.created_at || "?"}${next}`;
          card.appendChild(ts);
          const replay = document.createElement("button");
          replay.type = "button";
          replay.textContent = "Replay";
          replay.onclick = () => replayDelivery(d.id, d.kind);
          card.appendChild(replay);
          box.appendChild(card);
        });
      }


      async function loadDeliveries() {
        const box = document.getElementById("op-deliv-list");
        box.innerHTML = "";
        const status = document.getElementById("op-deliv-status").value;
        const kind = document.getElementById("op-deliv-kind").value;
        let query = `kind=${encodeURIComponent(kind)}&limit=50`;
        if (status === "quarantined") query += "&quarantined=true";
        else if (status === "delivered") query += "&delivered=true";
        const suffix = `/workspaces/${wid()}/deliveries?${query}`;
        const url = token() ? `${base()}${suffix}` : uiReadPath(suffix);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) {
            box.textContent = `HTTP ${res.status}`;
            return;
          }
          renderDeliveries(await res.json());
        } catch (e) {
          box.textContent = String(e);
        }
      }


      async function replayDelivery(id, kind) {
        if (!requireAuthForWrite()) return;
        try {
          const res = await api(
            apiWritePath(`/workspaces/${wid()}/deliveries/${id}/replay?kind=${encodeURIComponent(kind)}`),
            { method: "POST", headers: headers(true), credentials: "include" },
          );
          if (!res.ok) return setStatus(`replay HTTP ${res.status}`, "err");
          setStatus(`Replayed ${kind} delivery #${id}`, "ok");
          await loadDeliveries();
        } catch (e) {
          setStatus(String(e), "err");
        }
      }


      async function loadGlobalAudit() {
        const box = document.getElementById("op-audit-list");
        box.innerHTML = "";
        if (!token()) {
          box.textContent =
            "Set a bearer token with audit:read-global above to load the global audit.";
          return;
        }
        const limit = document.getElementById("op-audit-limit").value || "100";
        try {
          const res = await api(
            `${base()}/operator/audit?limit=${encodeURIComponent(limit)}`,
            { headers: headers(), credentials: "include" },
          );
          if (!res.ok) {
            box.textContent =
              res.status === 403
                ? "403 — token lacks audit:read-global"
                : `HTTP ${res.status}`;
            return;
          }
          const rows = await res.json();
          if (!rows.length) {
            box.textContent = "No audit events";
            return;
          }
          rows.forEach((e) => {
            const div = document.createElement("div");
            div.className = "deliv";
            const head = document.createElement("div");
            head.className = "deliv-head";
            head.textContent = `${e.occurred_at} · ${e.action}`;
            div.appendChild(head);
            const meta = document.createElement("div");
            meta.className = "muted";
            const actor = e.actor_id || "system";
            const target = e.target_kind
              ? `${e.target_kind}${e.target_id ? " " + e.target_id : ""}`
              : "—";
            meta.textContent = `actor: ${actor} · target: ${target}`;
            div.appendChild(meta);
            box.appendChild(div);
          });
        } catch (e) {
          box.textContent = String(e);
        }
      }


      function renderReindexJob(j) {
        const box = document.getElementById("op-reindex-status");
        const scope = j.workspace_id ? `workspace ${j.workspace_id}` : "system-wide";
        let line = `job ${j.job_id} · ${j.status} · ${scope} · model ${j.embedding_model}`;
        if (j.processed !== undefined) line += ` · processed ${j.processed}`;
        if (j.failed !== undefined) line += ` · failed ${j.failed}`;
        if (j.error) line += ` · error: ${j.error}`;
        box.textContent = line;
      }


      async function startReindex(global) {
        if (!requireAuthForWrite()) return;
        if (global && !token()) {
          return setStatus("System-wide reindex needs a token:admin bearer", "err");
        }
        const body = global ? {} : { workspace_id: wid() };
        try {
          const res = await api(apiWritePath(`/operator/reindex-embeddings`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify(body),
          });
          if (!res.ok) return setStatus(`reindex HTTP ${res.status}`, "err");
          const job = await res.json();
          renderReindexJob(job);
          document.getElementById("op-reindex-job").value = job.job_id;
          setStatus(`Reindex started (${job.job_id})`, "ok");
        } catch (e) {
          setStatus(String(e), "err");
        }
      }


      async function pollReindex() {
        const id = document.getElementById("op-reindex-job").value.trim();
        if (!id) return setStatus("Enter a job ID", "err");
        try {
          const res = await api(
            apiWritePath(`/operator/reindex-embeddings/${encodeURIComponent(id)}`),
            { headers: headers(), credentials: "include" },
          );
          if (!res.ok) return setStatus(`poll HTTP ${res.status}`, "err");
          renderReindexJob(await res.json());
        } catch (e) {
          setStatus(String(e), "err");
        }
      }


      async function loadSlashCommands() {
        const box = document.getElementById("slash-list");
        box.innerHTML = "";
        const suffix = `/workspaces/${wid()}/slash-commands`;
        const url = token() ? `${base()}${suffix}` : uiReadPath(suffix);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) {
            box.textContent = `HTTP ${res.status}`;
            return;
          }
          const cmds = await res.json();
          if (!cmds.length) {
            box.textContent = "No slash commands";
            return;
          }
          cmds.forEach((c) => {
            const div = document.createElement("div");
            div.className = "deliv";
            const head = document.createElement("div");
            head.className = "deliv-head";
            const tag = document.createElement("span");
            tag.className = "deliv-kind";
            tag.textContent = c.handler_kind;
            head.appendChild(tag);
            const name = document.createElement("span");
            name.textContent = ` /${c.name}${c.enabled ? "" : " · revoked"}`;
            head.appendChild(name);
            div.appendChild(head);
            if (c.description) {
              const d = document.createElement("div");
              d.textContent = c.description;
              div.appendChild(d);
            }
            const target = document.createElement("div");
            target.className = "muted";
            target.textContent = `→ ${c.handler_target}`;
            div.appendChild(target);
            if (c.enabled) {
              const revoke = document.createElement("button");
              revoke.type = "button";
              revoke.textContent = "Revoke";
              revoke.onclick = () => revokeSlashCommand(c.id);
              div.appendChild(revoke);
            }
            box.appendChild(div);
          });
        } catch (e) {
          box.textContent = String(e);
        }
      }


      async function registerSlashCommand() {
        if (!requireAuthForWrite()) return;
        const name = document.getElementById("slash-name").value.trim();
        if (!name) return showError("Name required");
        const handlerKind = document.getElementById("slash-kind").value;
        const handlerTarget = document.getElementById("slash-target").value.trim();
        if (!handlerTarget) return showError("Handler target required");
        const description = document.getElementById("slash-desc").value.trim() || null;
        const secretBox = document.getElementById("slash-secret");
        secretBox.innerHTML = "";
        try {
          const res = await api(apiWritePath(`/workspaces/${wid()}/slash-commands`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({
              name,
              description,
              handler_kind: handlerKind,
              handler_target: handlerTarget,
            }),
          });
          if (!res.ok) return setStatus(`register HTTP ${res.status}`, "err");
          const out = await res.json();
          setStatus(`Registered /${name}`, "ok");
          if (out.secret) {
            const warn = document.createElement("div");
            warn.textContent = "Signing secret (shown once — copy it now):";
            secretBox.appendChild(warn);
            const code = document.createElement("code");
            code.textContent = out.secret;
            code.classList.add("break-all");
            secretBox.appendChild(code);
            const copy = document.createElement("button");
            copy.type = "button";
            copy.textContent = "Copy";
            copy.onclick = () => navigator.clipboard.writeText(out.secret);
            secretBox.appendChild(copy);
          }
          document.getElementById("slash-name").value = "";
          document.getElementById("slash-target").value = "";
          document.getElementById("slash-desc").value = "";
          await loadSlashCommands();
        } catch (e) {
          setStatus(String(e), "err");
        }
      }


      async function revokeSlashCommand(id) {
        if (!requireAuthForWrite()) return;
        try {
          const res = await api(apiWritePath(`/workspaces/${wid()}/slash-commands/${id}`), {
            method: "DELETE",
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) return setStatus(`revoke HTTP ${res.status}`, "err");
          setStatus("Command revoked", "ok");
          await loadSlashCommands();
        } catch (e) {
          setStatus(String(e), "err");
        }
      }


      async function loadMessageEdits(messageId) {
        const box = document.getElementById("edit-history-list");
        if (!messageId) {
          box.textContent = "Select a message";
          return;
        }
        const url = `${uiReadPath(`/messages/${messageId}/edits`)}?limit=50`;
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          const body = await res.text();
          if (!res.ok) {
            box.textContent = body;
            return;
          }
          const edits = JSON.parse(body);
          box.innerHTML = "";
          if (!edits.length) {
            box.textContent = "No edit history (body unchanged or never edited)";
            return;
          }
          edits.forEach((e) => {
            const div = document.createElement("div");
            div.className = "row";
            div.innerHTML =
              `<span class="edited">${escapeHtml(e.edited_at)}</span> ` +
              `<span class="muted">${escapeHtml(e.editor_id)}</span><br>` +
              `<del>${escapeHtml(e.body_before)}</del> → ${escapeHtml(e.body_after)}`;
            box.appendChild(div);
          });
        } catch (e) {
          box.textContent = String(e);
        }
      }


      async function loadNotifications() {
        const list = document.getElementById("notification-list");
        const badge = document.getElementById("notif-unread");
        if (!sessionMemberId) {
          list.innerHTML =
            "<li class='muted'>Sign in (session) to see your notifications.</li>";
          badge.textContent = "";
          return;
        }
        const unreadOnly = document.getElementById("notif-unread-only").checked;
        try {
          const cres = await api(
            uiReadPath(`/members/${sessionMemberId}/notifications/unread-count`),
            { headers: headers(), credentials: "include" },
          );
          badge.textContent = cres.ok
            ? `— ${(await cres.json()).count} unread`
            : "";
          const suffix = `/members/${sessionMemberId}/notifications?unread_only=${unreadOnly}&limit=50`;
          const res = await api(uiReadPath(suffix), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            list.innerHTML = `<li class='muted'>Failed to load (HTTP ${res.status}).</li>`;
            return;
          }
          renderNotifications(await res.json());
        } catch (e) {
          list.innerHTML = `<li class='muted'>Error: ${escapeHtml(String(e))}</li>`;
        }
      }


      function renderNotifications(items) {
        const list = document.getElementById("notification-list");
        list.innerHTML = "";
        if (!items.length) {
          list.innerHTML = "<li class='muted'>No notifications.</li>";
          return;
        }
        for (const n of items) {
          const li = document.createElement("li");
          const where = n.thread_id ? ` in thread ${n.thread_id}` : "";
          const readMark = n.read_at ? " (read)" : "";
          li.textContent = `${n.kind}${where}${readMark}`;
          if (!n.read_at) {
            const btn = document.createElement("button");
            btn.type = "button";
            btn.textContent = "Mark read";
            btn.classList.add("gap-left");
            btn.onclick = () => markNotificationRead(n.id);
            li.appendChild(btn);
          }
          list.appendChild(li);
        }
      }


      async function markNotificationRead(nid) {
        if (!requireAuthForWrite()) return;
        try {
          const res = await api(
            apiWritePath(`/members/${sessionMemberId}/notifications/${nid}/read`),
            { method: "POST", headers: headers(), credentials: "include" },
          );
          if (!res.ok) setStatus(`HTTP ${res.status}`, "err");
        } catch (e) {
          setStatus(String(e), "err");
        }
        await loadNotifications();
      }


      // --- Work tab: the work-observability console. ---
      async function loadWork() {
        await loadWaiting();
        await loadWorkChannels();
        await loadWorkSchedules();
      }


      // Waiting-on-you inbox.
      async function loadWaiting() {
        const list = document.getElementById("waiting-list");
        const summary = document.getElementById("waiting-summary");
        const id = authorId();
        if (!id) {
          list.innerHTML =
            "<li class='muted'>Sign in or set a valid bearer token.</li>";
          summary.textContent = "";
          return;
        }
        const sla = document.getElementById("waiting-sla").value.trim() || "86400";
        try {
          const res = await api(
            uiReadPath(`/members/${id}/waiting?sla_secs=${encodeURIComponent(sla)}`),
            { headers: headers(), credentials: "include" },
          );
          if (!res.ok) {
            list.innerHTML = `<li class='muted'>Failed (HTTP ${res.status}).</li>`;
            summary.textContent = "";
            return;
          }
          const inbox = await res.json();
          summary.textContent = `${inbox.total} waiting, ${inbox.overdue} overdue`;
          list.innerHTML = "";
          if (!inbox.items.length) {
            list.innerHTML = "<li class='muted'>Nothing waiting on you. 🎉</li>";
            return;
          }
          for (const it of inbox.items) {
            const li = document.createElement("li");
            const hrs = Math.floor(it.age_secs / 3600);
            li.textContent = `[${it.kind}] ${it.summary} — ${hrs}h${it.overdue ? " ⚠ overdue" : ""}`;
            if (it.overdue) li.className = "err";
            list.appendChild(li);
          }
        } catch (e) {
          list.innerHTML = `<li class='muted'>Error: ${escapeHtml(String(e))}</li>`;
        }
      }


      async function loadWorkChannels() {
        const sel = document.getElementById("work-channel");
        const ws = document.getElementById("workspace").value.trim();
        if (!ws) {
          sel.innerHTML = "<option value=''>— set a workspace above —</option>";
          return;
        }
        try {
          const res = await api(uiReadPath(`/workspaces/${ws}/channels`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            sel.innerHTML = `<option value=''>load failed (HTTP ${res.status})</option>`;
            return;
          }
          const chans = await res.json();
          const prev = sel.value;
          sel.innerHTML = "<option value=''>— select a channel —</option>";
          for (const c of chans) {
            const o = document.createElement("option");
            o.value = c.id;
            o.textContent = c.name;
            sel.appendChild(o);
          }
          if (prev) sel.value = prev;
        } catch (e) {
          sel.innerHTML = "<option value=''>error</option>";
        }
      }


      async function loadWorkDepth() {
        const el = document.getElementById("work-depth");
        const cid = document.getElementById("work-channel").value;
        if (!cid) {
          el.textContent = "Select a channel to load its queue.";
          return;
        }
        try {
          const [dres, ores] = await Promise.all([
            api(uiReadPath(`/channels/${cid}/queue-depth`), {
              headers: headers(),
              credentials: "include",
            }),
            api(uiReadPath(`/channels/${cid}/occupancy`), {
              headers: headers(),
              credentials: "include",
            }),
          ]);
          if (!dres.ok) {
            el.textContent = `Queue depth failed (HTTP ${dres.status}).`;
            return;
          }
          const d = await dres.json();
          let occ = "";
          if (ores.ok) {
            const o = await ores.json();
            occ = ` · occupancy — queued ${o.queued}, claimed ${o.claimed}, working ${o.working}, blocked ${o.blocked}`;
          }
          el.textContent = `Queue — open ${d.open}, ready ${d.ready}, assigned ${d.assigned}, blocked ${d.blocked}, unclaimable ${d.unclaimable}${occ}`;
        } catch (e) {
          el.textContent = `Error: ${e}`;
        }
      }


      async function loadWorkThreads() {
        const list = document.getElementById("work-thread-list");
        const cid = document.getElementById("work-channel").value;
        document.getElementById("work-thread-detail").innerHTML = "";
        if (!cid) {
          list.innerHTML = "<li class='muted'>Select a channel.</li>";
          return;
        }
        try {
          const res = await api(uiReadPath(`/channels/${cid}/threads`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            list.innerHTML = `<li class='muted'>Failed (HTTP ${res.status}).</li>`;
            return;
          }
          const threads = await res.json();
          list.innerHTML = "";
          if (!threads.length) {
            list.innerHTML = "<li class='muted'>No threads.</li>";
            return;
          }
          for (const t of threads) {
            const li = document.createElement("li");
            li.textContent = `${t.title || "(untitled)"} — ${t.state}`;
            const btn = document.createElement("button");
            btn.type = "button";
            btn.textContent = "Inspect";
            btn.classList.add("gap-left");
            btn.onclick = () => showWorkThread(t.id);
            li.appendChild(btn);
            list.appendChild(li);
          }
        } catch (e) {
          list.innerHTML = `<li class='muted'>Error: ${escapeHtml(String(e))}</li>`;
        }
      }


      async function showWorkThread(tid) {
        const el = document.getElementById("work-thread-detail");
        el.innerHTML = "Loading…";
        try {
          const [rres, dres] = await Promise.all([
            api(uiReadPath(`/threads/${tid}/result`), {
              headers: headers(),
              credentials: "include",
            }),
            api(uiReadPath(`/threads/${tid}/dependencies`), {
              headers: headers(),
              credentials: "include",
            }),
          ]);
          let html = `<h4>Thread ${escapeHtml(tid)}</h4>`;
          if (rres.ok) {
            const r = await rres.json();
            html += `<p><strong>Result</strong> (by ${escapeHtml(r.produced_by)}): <code>${escapeHtml(JSON.stringify(r.result))}</code></p>`;
          } else if (rres.status === 404) {
            html += "<p class='muted'>No result recorded yet.</p>";
          } else {
            html += `<p class='muted'>Result load failed (HTTP ${rres.status}).</p>`;
          }
          if (dres.ok) {
            const d = await dres.json();
            const deps =
              (d.dependencies || [])
                .map((x) => escapeHtml(x.depends_on_thread_id))
                .join(", ") || "none";
            html += `<p><strong>Depends on:</strong> ${deps} · <strong>ready:</strong> ${d.ready}</p>`;
          }
          el.innerHTML = html;
        } catch (e) {
          el.innerHTML = `<p class='muted'>Error: ${escapeHtml(String(e))}</p>`;
        }
      }


      async function loadWorkSchedules() {
        const list = document.getElementById("work-schedule-list");
        const ws = document.getElementById("workspace").value.trim();
        if (!ws) {
          list.innerHTML = "<li class='muted'>Set a workspace above.</li>";
          return;
        }
        try {
          const res = await api(uiReadPath(`/workspaces/${ws}/task-schedules`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            list.innerHTML = `<li class='muted'>Failed (HTTP ${res.status}).</li>`;
            return;
          }
          const scheds = await res.json();
          list.innerHTML = "";
          if (!scheds.length) {
            list.innerHTML = "<li class='muted'>No schedules.</li>";
            return;
          }
          for (const s of scheds) {
            const li = document.createElement("li");
            const cadence = s.interval_secs ? `every ${s.interval_secs}s` : "one-shot";
            li.textContent = `${s.title} — ${cadence} (${s.active ? "active" : "paused"}), next ${s.next_run_at}`;
            list.appendChild(li);
          }
        } catch (e) {
          list.innerHTML = `<li class='muted'>Error: ${escapeHtml(String(e))}</li>`;
        }
      }


      // --- Prefs console: notification prefs, delivery,
      // email, and follows — all self-only (the acting session member). ---
      async function loadPrefs() {
        const el = document.getElementById("prefs-delivery");
        if (!sessionMemberId) {
          el.textContent = "Sign in (session) to manage preferences.";
          return;
        }
        const id = sessionMemberId;
        try {
          const res = await api(uiReadPath(`/members/${id}/delivery-mode`), {
            headers: headers(),
            credentials: "include",
          });
          el.textContent = res.ok
            ? `Delivery mode: ${(await res.json()).mode}`
            : "";
        } catch (e) {
          el.textContent = "";
        }
        try {
          const res = await api(uiReadPath(`/members/${id}/email`), {
            headers: headers(),
            credentials: "include",
          });
          document.getElementById("prefs-email-current").textContent = res.ok
            ? `current: ${(await res.json()).email}`
            : "(none set)";
        } catch (e) {
          /* ignore */
        }
        try {
          const res = await api(uiReadPath(`/members/${id}/notification-prefs`), {
            headers: headers(),
            credentials: "include",
          });
          const list = document.getElementById("prefs-mute-list");
          list.innerHTML = "";
          if (res.ok) {
            const muted = (await res.json()).filter((p) => p.muted);
            if (!muted.length) {
              list.innerHTML = "<li class='muted'>No muted kinds.</li>";
            }
            for (const p of muted) {
              const li = document.createElement("li");
              li.textContent = p.kind;
              list.appendChild(li);
            }
          }
        } catch (e) {
          /* ignore */
        }
        await loadPrefsFollows("channel-follows", "prefs-channel-follows", "channel_id");
        await loadPrefsFollows("thread-follows", "prefs-thread-follows", "thread_id");
      }


      async function loadPrefsFollows(path, listId, idField) {
        const list = document.getElementById(listId);
        list.innerHTML = "";
        try {
          const res = await api(
            uiReadPath(`/members/${sessionMemberId}/${path}`),
            { headers: headers(), credentials: "include" },
          );
          if (!res.ok) {
            list.innerHTML = "<li class='muted'>load failed</li>";
            return;
          }
          const follows = await res.json();
          if (!follows.length) {
            list.innerHTML = "<li class='muted'>None.</li>";
            return;
          }
          for (const f of follows) {
            const targetId = f[idField];
            const li = document.createElement("li");
            li.textContent = targetId + " ";
            const btn = document.createElement("button");
            btn.type = "button";
            btn.textContent = "Unfollow";
            btn.onclick = () => unfollowTarget(path, targetId);
            li.appendChild(btn);
            list.appendChild(li);
          }
        } catch (e) {
          list.innerHTML = "<li class='muted'>error</li>";
        }
      }


      async function prefsWrite(method, suffix, body) {
        if (!requireAuthForWrite()) return null;
        try {
          /** @type {RequestInit} */
          const opts = { method, headers: headers(!!body), credentials: "include" };
          if (body) opts.body = JSON.stringify(body);
          const res = await api(apiWritePath(suffix), opts);
          if (!res.ok) {
            setStatus(`HTTP ${res.status}`, "err");
            return null;
          }
          return res;
        } catch (e) {
          setStatus(String(e), "err");
          return null;
        }
      }


      async function setPrefsDeliveryMode(mode) {
        await prefsWrite("PUT", `/members/${sessionMemberId}/delivery-mode`, { mode });
        await loadPrefs();
      }

      async function setPrefsEmail() {
        const email = document.getElementById("prefs-email").value.trim();
        if (!email) return;
        await prefsWrite("PUT", `/members/${sessionMemberId}/email`, { email });
        await loadPrefs();
      }

      async function clearPrefsEmail() {
        await prefsWrite("DELETE", `/members/${sessionMemberId}/email`, null);
        await loadPrefs();
      }

      async function setPrefsMute(muted) {
        const kind = document.getElementById("prefs-mute-kind").value;
        await prefsWrite("PUT", `/members/${sessionMemberId}/notification-prefs`, {
          kind,
          muted,
        });
        await loadPrefs();
      }

      async function followTarget(path) {
        const field = path === "channel-follows" ? "channel" : "thread";
        const val = document.getElementById(`prefs-follow-${field}`).value.trim();
        if (!val) return;
        const body =
          path === "channel-follows" ? { channel_id: val } : { thread_id: val };
        await prefsWrite("POST", `/members/${sessionMemberId}/${path}`, body);
        await loadPrefs();
      }

      async function unfollowTarget(path, targetId) {
        await prefsWrite(
          "DELETE",
          `/members/${sessionMemberId}/${path}/${targetId}`,
          null,
        );
        await loadPrefs();
      }


      // --- Looking glass: kind / thread / sha / peer
      // read-only explorer. Each section is button-driven. ---
      function loadGlass() {
        /* on-demand — each section loads on its own button */
      }


      async function glassEventsByKind() {
        const list = document.getElementById("glass-events");
        const ws = document.getElementById("workspace").value.trim();
        const kind = document.getElementById("glass-kind").value;
        if (!ws) {
          list.innerHTML = "<li class='muted'>Set a workspace above.</li>";
          return;
        }
        try {
          const res = await api(uiReadPath(`/workspaces/${ws}/events?limit=200`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            list.innerHTML = `<li class='muted'>Failed (HTTP ${res.status}).</li>`;
            return;
          }
          const events = (await res.json()).filter((e) => e.kind === kind);
          list.innerHTML = "";
          if (!events.length) {
            list.innerHTML = `<li class='muted'>No recent ${escapeHtml(kind)} events.</li>`;
            return;
          }
          for (const e of events.slice(-50).reverse()) {
            const li = document.createElement("li");
            const where = e.thread_id
              ? ` thread ${e.thread_id}`
              : e.channel_id
                ? ` channel ${e.channel_id}`
                : "";
            li.textContent = `#${e.id} ${e.kind}${where}`;
            list.appendChild(li);
          }
        } catch (e) {
          list.innerHTML = `<li class='muted'>Error: ${escapeHtml(String(e))}</li>`;
        }
      }


      async function glassThread() {
        const list = document.getElementById("glass-thread-messages");
        const tid = document.getElementById("glass-thread").value.trim();
        if (!tid) return;
        try {
          const res = await api(uiReadPath(`/threads/${tid}/messages?limit=50`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            list.innerHTML = `<li class='muted'>Failed (HTTP ${res.status}).</li>`;
            return;
          }
          const msgs = await res.json();
          list.innerHTML = "";
          if (!msgs.length) {
            list.innerHTML = "<li class='muted'>No messages.</li>";
            return;
          }
          for (const m of msgs) {
            const li = document.createElement("li");
            li.textContent = `${memberName(m.author_id)}: ${m.body}`;
            list.appendChild(li);
          }
        } catch (e) {
          list.innerHTML = `<li class='muted'>Error: ${escapeHtml(String(e))}</li>`;
        }
      }


      async function glassArtifact() {
        const el = document.getElementById("glass-artifact");
        const sha = document.getElementById("glass-sha").value.trim();
        if (!sha) return;
        el.textContent = "Looking up…";
        try {
          const res = await api(uiReadPath(`/artifacts/${sha}/meta`), {
            headers: headers(),
            credentials: "include",
          });
          if (res.status === 404) {
            el.textContent = "Not found (or not in this workspace).";
            return;
          }
          if (!res.ok) {
            el.textContent = `Failed (HTTP ${res.status}).`;
            return;
          }
          el.textContent = JSON.stringify(await res.json(), null, 2);
        } catch (e) {
          el.textContent = `Error: ${e}`;
        }
      }


      async function glassPeers() {
        const list = document.getElementById("glass-peers");
        const ws = document.getElementById("workspace").value.trim();
        if (!ws) {
          list.innerHTML = "<li class='muted'>Set a workspace above.</li>";
          return;
        }
        try {
          const res = await api(uiReadPath(`/workspaces/${ws}/peers`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            list.innerHTML = `<li class='muted'>Failed (HTTP ${res.status}).</li>`;
            return;
          }
          const peers = await res.json();
          list.innerHTML = "";
          if (!peers.length) {
            list.innerHTML = "<li class='muted'>No peers.</li>";
            return;
          }
          for (const p of peers) {
            const li = document.createElement("li");
            li.textContent = `${p.name ?? p.id} — ${p.base_url ?? ""}`;
            list.appendChild(li);
          }
        } catch (e) {
          list.innerHTML = `<li class='muted'>Error: ${escapeHtml(String(e))}</li>`;
        }
      }


      async function loadApprovals(showLoading = true) {
        const list = document.getElementById("approval-list");
        const ws = document.getElementById("workspace").value.trim();
        if (!ws) {
          renderState(list, "Set a workspace ID above to watch pending approvals.");
          return;
        }
        if (approvalsLoading) return;
        approvalsLoading = true;
        if (showLoading) setLoading(list, "Checking for pending approvals…");
        try {
          const res = await api(uiReadPath(`/workspaces/${ws}/approval-gates`), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            const message = await responseError(res, "Could not load approvals");
            setStatus(message, "err");
            renderState(list, message, "err");
            return;
          }
          renderApprovals(await res.json());
        } catch (e) {
          renderState(list, String(e), "err");
        } finally {
          approvalsLoading = false;
          clearLoading(list);
        }
      }


      function renderApprovals(views) {
        const list = document.getElementById("approval-list");
        list.innerHTML = "";
        if (!views.length) {
          renderState(list, "You're caught up — no pending approval gates.");
          return;
        }
        list.classList.remove("muted");
        for (const v of views) {
          const gate = v.gate;
          const li = document.createElement("li");
          li.className = "approval-row";
          li.dataset.gateId = gate.id;
          const prompt = document.createElement("span");
          prompt.textContent = gate.prompt;
          li.appendChild(prompt);
          for (const action of ["accept", "decline", "cancel"]) {
            const btn = document.createElement("button");
            btn.type = "button";
            btn.textContent = action;
            btn.className = `gate-${action}`;
            btn.onclick = () => answerGate(gate.id, action, v.request_state);
            li.appendChild(btn);
          }
          list.appendChild(li);
        }
      }


      // Answers a gate and says whether it took: { ok } or { ok: false, why }.
      async function answerGate(gateId, action, requestState) {
        if (!requireAuthForWrite()) return { ok: false, why: "Sign in or paste a token first." };
        let res;
        try {
          res = await api(apiWritePath(`/approval-gates/${gateId}/answer`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ request_state: requestState, action }),
          });
        } catch (e) {
          const why = unreachable(e);
          setStatus(why, "err");
          return { ok: false, why };
        }
        if (!res.ok) {
          const why = await responseError(res, `Could not ${action} gate`);
          setStatus(why, "err");
          return { ok: false, why };
        }
        setStatus(`gate ${action}ed`, "ok");
        await loadApprovals();
        return { ok: true };
      }


      async function loadSession() {
        try {
          const res = await api(uiReadPath("/me"), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            setStatus(`HTTP ${res.status}`, "err");
            document.getElementById("session-member").textContent =
              `HTTP ${res.status} — sign in or set a bearer token above`;
            return;
          }
          renderSession(await res.json());
        } catch (e) {
          setStatus(String(e), "err");
        }
      }


      // The caller's own capabilities — the attenuation ceiling for minting.
      // Cached from /me so the mint pre-flight can flag a widening request.
      let myCapabilities = null;

      let currentTokenId = null;


      async function loadAttenuationCeiling() {
        const span = document.getElementById("attenuation-ceiling");
        try {
          const res = await api(uiReadPath("/me"), {
            headers: headers(),
            credentials: "include",
          });
          if (!res.ok) {
            span.textContent = `unknown (HTTP ${res.status})`;
            return;
          }
          const me = await res.json();
          myCapabilities = me.capabilities || [];
          span.textContent = myCapabilities.length
            ? myCapabilities.slice().sort().join(", ")
            : "none";
        } catch (e) {
          span.textContent = "unknown";
        }
      }


      // Requested caps that exceed the caller's own grant (the widening set).
      // Empty when the request is a valid attenuation (a subset).
      function capsExceedingGrant(requested) {
        if (!myCapabilities) return [];
        const grant = new Set(myCapabilities);
        return requested.filter((c) => !grant.has(c));
      }


      function renderSession(me) {
        myCapabilities = me.capabilities || [];
        currentTokenId = me.token_id || null;
        document.getElementById("session-member").textContent = me.member_id;
        document.getElementById("session-workspace").textContent = me.workspace_id;
        // A token acts as exactly one member; acting for another is a
        // delegation grant, which the token then names.
        document.getElementById("session-credential").textContent = !me.is_bearer
          ? "signed-in session — pinned to this member"
          : me.delegation_grant_id
            ? "delegated token — works as this member under a grant"
            : "bearer token — acts as this member";
        document.getElementById("session-rotate").hidden = !(currentTokenId && token());
        const granted = new Set(me.capabilities || []);
        const known = me.known_capabilities || [];
        const can = [...granted].sort();
        const cant = known.filter((c) => !granted.has(c)).sort();
        renderCapList("cap-can-list", "cap-can-count", can, "No capabilities granted.");
        renderCapList(
          "cap-cant-list",
          "cap-cant-count",
          cant,
          "Nothing withheld — the full vocabulary is granted.",
        );
      }


      function renderCapList(listId, countId, caps, emptyMsg) {
        const list = document.getElementById(listId);
        list.innerHTML = "";
        document.getElementById(countId).textContent = `(${caps.length})`;
        if (!caps.length) {
          list.classList.add("muted");
          const li = document.createElement("li");
          li.textContent = emptyMsg;
          list.appendChild(li);
          return;
        }
        list.classList.remove("muted");
        for (const c of caps) {
          const li = document.createElement("li");
          li.textContent = c;
          list.appendChild(li);
        }
      }


      async function markAllNotificationsRead() {
        if (!requireAuthForWrite()) return;
        try {
          const res = await api(
            apiWritePath(`/members/${sessionMemberId}/notifications/read-all`),
            { method: "POST", headers: headers(), credentials: "include" },
          );
          if (!res.ok) setStatus(`HTTP ${res.status}`, "err");
        } catch (e) {
          setStatus(String(e), "err");
        }
        await loadNotifications();
      }


      // ARIA tablist semantics + keyboard operability (WCAG 2.1.1 / 4.1.2):
      // link tabs↔panels, rove tabindex so only the selected tab is in the tab
      // order, and support Arrow/Home/End navigation across the tablist.
      function initTablist() {
        const tabs = Array.from(document.querySelectorAll(".tabs button[data-tab]"));
        tabs.forEach((btn) => {
          const name = btn.dataset.tab;
          btn.setAttribute("role", "tab");
          btn.id = `tab-${name}`;
          btn.setAttribute("aria-controls", `panel-${name}`);
          btn.tabIndex = btn.getAttribute("aria-selected") === "true" ? 0 : -1;
          const panel = document.getElementById(`panel-${name}`);
          if (panel) {
            panel.setAttribute("role", "tabpanel");
            panel.setAttribute("aria-labelledby", `tab-${name}`);
            panel.tabIndex = 0;
          }
        });
        const tablist = document.querySelector(".tabs");
        if (!tablist) return;
        tablist.addEventListener("keydown", (e) => {
          const idx = tabs.indexOf(document.activeElement);
          if (idx < 0) return;
          let next = null;
          if (e.key === "ArrowRight" || e.key === "ArrowDown") next = (idx + 1) % tabs.length;
          else if (e.key === "ArrowLeft" || e.key === "ArrowUp")
            next = (idx - 1 + tabs.length) % tabs.length;
          else if (e.key === "Home") next = 0;
          else if (e.key === "End") next = tabs.length - 1;
          if (next === null) return;
          e.preventDefault();
          tabs[next].focus();
          tabs[next].click();
        });
      }

      // Rotating the token this page runs on ends the session made from it, so
      // the successor is exchanged before the live socket reconnects.
      async function rotateToken(id) {
        const res = await api(`${base()}/tokens/${encodeURIComponent(id)}/rotate`, {
          method: "POST",
          headers: headers(),
        });
        if (!res.ok) {
          showError(await responseError(res, "Could not rotate that token"));
          return;
        }
        const rotated = await res.json();
        if (id === currentTokenId) {
          currentTokenId = rotated.id;
          const exchanged = await exchangeToken(rotated.secret);
          if (!exchanged.ok) {
            document.getElementById("token").value = rotated.secret;
            showError(exchanged.error);
          }
          persist();
          if (wsSocket) {
            disconnectWs();
            connectWs();
          }
        }
        showSecretOnce("New token (shown once) — the old one no longer works", rotated.secret);
        setStatus("Token rotated", "ok");
        setOut({ rotated: id, id: rotated.id });
      }

      function parseCaps(raw) {
        return raw
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean);
      }


      async function loadPeers() {
        const box = document.getElementById("peer-list");
        box.innerHTML = "";
        if (!wid()) {
          box.textContent = "Set workspace ID";
          return;
        }
        persist();
        const res = await api(uiReadPath(`/workspaces/${wid()}/peers`), {
          headers: headers(),
          credentials: "include",
        });
        const body = await res.text();
        if (!res.ok) {
          box.textContent = body;
          return;
        }
        const peers = JSON.parse(body);
        if (!peers.length) {
          box.textContent = "No federation peers";
          return;
        }
        peers.forEach((p) => {
          const div = document.createElement("div");
          div.className = "row";
          div.textContent = `${p.name} · ${p.id} · ${p.base_url} · enabled=${p.enabled}`;
          box.appendChild(div);
        });
      }

export { answerGate, approvalsLoading, capsExceedingGrant, clearPrefsEmail, currentTokenId, followTarget, glassArtifact, glassEventsByKind, glassPeers, glassThread, initTablist, loadApprovals, loadAttenuationCeiling, loadDeliveries, loadGlass, loadGlobalAudit, loadMessageEdits, loadNotifications, loadPeers, loadPrefs, loadPrefsFollows, loadSession, loadSlashCommands, loadWaiting, loadWork, loadWorkChannels, loadWorkDepth, loadWorkSchedules, loadWorkThreads, markAllNotificationsRead, markNotificationRead, myCapabilities, parseCaps, pollReindex, prefsWrite, registerSlashCommand, renderApprovals, renderCapList, renderDeliveries, renderNotifications, renderReindexJob, renderSession, replayDelivery, revokeSlashCommand, rotateToken, setPrefsDeliveryMode, setPrefsEmail, setPrefsMute, showWorkThread, startReindex, unfollowTarget };
