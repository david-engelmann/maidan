// @ts-check
import { api, apiWritePath, base, headers, requireAuthForWrite, token, uiReadPath, wid, writeApi } from "./api.js";
import { renderState, responseError, showError, unreachable } from "./feedback.js";
import { authorId, loadMembers, memberName } from "./people.js";


      let selectedGdm = null;

      let selectedDm = null;


      function dmOther(c) {
        const me = authorId();
        return c.member_low_id === me ? c.member_high_id : c.member_low_id;
      }


      async function loadDms() {
        const list = document.getElementById("dm-list");
        list.innerHTML = "";
        const me = authorId();
        if (!me) {
          list.innerHTML = "<li>Sign in or set a valid bearer token</li>";
          return;
        }
        await loadMembers();
        const suffix = `/workspaces/${wid()}/dm?member_id=${encodeURIComponent(me)}`;
        const url = token() ? `${base()}${suffix}` : uiReadPath(suffix);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) {
            renderState(list, await responseError(res), "err");
            return;
          }
          const convos = await res.json();
          if (!convos.length) {
            list.innerHTML = "<li>No DMs</li>";
            return;
          }
          convos.forEach((c) => {
            const li = document.createElement("li");
            const a = document.createElement("a");
            a.href = "#";
            const other = dmOther(c);
            a.textContent = `with ${memberName(other)}`;
            a.title = other;
            a.onclick = (ev) => {
              ev.preventDefault();
              selectDm(c);
            };
            li.appendChild(a);
            list.appendChild(li);
          });
        } catch (e) {
          renderState(list, unreachable(e), "err");
        }
      }


      function selectDm(c) {
        selectedDm = c;
        document.getElementById("dm-selected").textContent = `with ${memberName(dmOther(c))}`;
        loadDmMessages(c.thread_id);
      }


      
      async function loadConversationMessages(threadId, boxId) {
        const box = document.getElementById(boxId);
        box.innerHTML = "";
        if (!threadId) return;
        const suffix = `/threads/${threadId}/messages?limit=50`;
        const url = token() ? `${base()}${suffix}` : uiReadPath(suffix);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) {
            box.textContent = await responseError(res);
            return;
          }
          const msgs = await res.json();
          if (!msgs.length) {
            box.textContent = "No messages";
            return;
          }
          msgs.forEach((m) => {
            const div = document.createElement("div");
            div.className = "msg";
            div.textContent = `${memberName(m.author_id)}: ${m.body}`;
            box.appendChild(div);
          });
        } catch (e) {
          box.textContent = unreachable(e);
        }
      }
      async function loadDmMessages(threadId) {
        return loadConversationMessages(threadId, "dm-messages");
      }
      async function loadGroupDmMessages(threadId) {
        return loadConversationMessages(threadId, "gdm-messages");
      }


      async function openDm() {
        if (!requireAuthForWrite()) return;
        const me = authorId();
        if (!me) return showError("Sign in or set a valid bearer token");
        const other = document.getElementById("dm-other-id").value.trim();
        if (!other) return showError("Enter the other member's ID");
        if (other === me) return showError("Cannot open a DM with yourself");
        try {
          const res = await writeApi("dm-open", apiWritePath(`/workspaces/${wid()}/dm`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ other_member_id: other }),
          });
          if (!res.ok) {
            showError(await responseError(res, "Could not open that DM"));
            return;
          }
          const opened = await res.json();
          showError("DM opened", "success");
          await loadMembers();
          await loadDms();
          selectDm(opened);
        } catch (e) {
          showError(unreachable(e));
        }
      }


      async function sendDmMessage() {
        if (!requireAuthForWrite()) return;
        const me = authorId();
        if (!me) return showError("Sign in or set a valid bearer token");
        if (!selectedDm) return showError("Select a DM first");
        const body = document.getElementById("dm-body").value.trim();
        if (!body) return showError("Message body required");
        try {
          const res = await writeApi("dm-send", apiWritePath(`/dm/${selectedDm.id}/messages`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ body }),
          });
          if (!res.ok) {
            showError(await responseError(res));
            return;
          }
          document.getElementById("dm-body").value = "";
          await loadDmMessages(selectedDm.thread_id);
        } catch (e) {
          showError(unreachable(e));
        }
      }


      async function loadGroupDms() {
        const list = document.getElementById("gdm-list");
        list.innerHTML = "";
        const me = authorId();
        if (!me) {
          list.innerHTML = "<li>Sign in or set a valid bearer token</li>";
          return;
        }
        const suffix = `/workspaces/${wid()}/group-dms?member_id=${encodeURIComponent(me)}`;
        const url = token() ? `${base()}${suffix}` : uiReadPath(suffix);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) {
            renderState(list, await responseError(res), "err");
            return;
          }
          const convos = await res.json();
          if (!convos.length) {
            list.innerHTML = "<li>No group DMs</li>";
            return;
          }
          convos.forEach((c) => {
            const li = document.createElement("li");
            const a = document.createElement("a");
            a.href = "#";
            a.textContent = `${c.title || "(untitled)"} · ${c.id.slice(0, 8)}…`;
            a.onclick = (ev) => {
              ev.preventDefault();
              selectGroupDm(c);
            };
            li.appendChild(a);
            list.appendChild(li);
          });
        } catch (e) {
          renderState(list, unreachable(e), "err");
        }
      }


      function selectGroupDm(c) {
        selectedGdm = c;
        document.getElementById("gdm-selected").textContent =
          `${c.title || "(untitled)"} · ${c.id}`;
        loadGroupDmMessages(c.thread_id);
      }


      


      async function openGroupDm() {
        if (!requireAuthForWrite()) return;
        const me = authorId();
        if (!me) return showError("Sign in or set a valid bearer token");
        const ids = document
          .getElementById("gdm-member-ids")
          .value.split(",")
          .map((s) => s.trim())
          .filter(Boolean);
        if (!ids.includes(me)) ids.push(me);
        // The store refuses fewer than three members. Say so here, before the request.
        if (ids.length < 3) return showError("A group DM needs at least 3 members");
        const title = document.getElementById("gdm-title").value.trim() || null;
        try {
          const res = await writeApi("gdm-open", apiWritePath(`/workspaces/${wid()}/group-dms`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ member_ids: ids, title }),
          });
          if (!res.ok) {
            showError(await responseError(res, "Could not open that group DM"));
            return;
          }
          const opened = await res.json();
          showError("Group DM opened", "success");
          await loadMembers();
          await loadGroupDms();
          selectGroupDm(opened);
        } catch (e) {
          showError(unreachable(e));
        }
      }


      async function sendGroupDmMessage() {
        if (!requireAuthForWrite()) return;
        const me = authorId();
        if (!me) return showError("Sign in or set a valid bearer token");
        if (!selectedGdm) return showError("Select a group DM first");
        const body = document.getElementById("gdm-body").value.trim();
        if (!body) return showError("Message body required");
        try {
          const res = await writeApi("gdm-send", apiWritePath(`/group-dms/${selectedGdm.id}/messages`), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ body }),
          });
          if (!res.ok) {
            showError(await responseError(res));
            return;
          }
          document.getElementById("gdm-body").value = "";
          await loadGroupDmMessages(selectedGdm.thread_id);
        } catch (e) {
          showError(unreachable(e));
        }
      }

export { dmOther, loadConversationMessages, loadDmMessages, loadDms, loadGroupDmMessages, loadGroupDms, openDm, openGroupDm, selectDm, selectGroupDm, selectedDm, selectedGdm, sendDmMessage, sendGroupDmMessage };
