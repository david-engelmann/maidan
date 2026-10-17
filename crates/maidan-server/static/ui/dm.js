// @ts-check
import { api, apiWritePath, base, headers, requireAuthForWrite, token, uiReadPath, wid, writeApi } from "./api.js";
import { renderState, responseError, showError, unreachable } from "./feedback.js";
import { authorId, loadMembers, memberName, memberPicker } from "./people.js";

      // A DM and a group DM are the same conversation on the page: a list, a
      // selection, a message pane and a composer. What differs is in the kind:
      // its elements, its routes and its words. One set of functions serves
      // both, so a fix to one cannot miss the other.
      /**
       * @typedef {object} ConversationKind
       * @property {string} list
       * @property {string} header
       * @property {string} messages
       * @property {string} body
       * @property {string} sendButton
       * @property {(me: string) => string} listPath
       * @property {(c: any) => string} sendPath
       * @property {boolean} namesMembers the list shows member names, so it needs the roster first
       * @property {string} none
       * @property {string} selectFirst
       * @property {(c: any) => string} label
       * @property {(c: any) => string} title
       * @property {(c: any) => string} selectedText
       * @property {any} selected
       */

      /** @type {ConversationKind} */
      const DM = {
        list: "dm-list",
        header: "dm-selected",
        messages: "dm-messages",
        body: "dm-body",
        sendButton: "dm-send",
        listPath: (me) => `/workspaces/${wid()}/dm?member_id=${encodeURIComponent(me)}`,
        sendPath: (c) => `/dm/${c.id}/messages`,
        namesMembers: true,
        none: "No DMs",
        selectFirst: "Select a DM first",
        label: (c) => `with ${memberName(dmOther(c))}`,
        title: (c) => dmOther(c),
        selectedText: (c) => `with ${memberName(dmOther(c))}`,
        selected: null,
      };

      /** @type {ConversationKind} */
      const GROUP_DM = {
        list: "gdm-list",
        header: "gdm-selected",
        messages: "gdm-messages",
        body: "gdm-body",
        sendButton: "gdm-send",
        listPath: (me) => `/workspaces/${wid()}/group-dms?member_id=${encodeURIComponent(me)}`,
        sendPath: (c) => `/group-dms/${c.id}/messages`,
        namesMembers: false,
        none: "No group DMs",
        selectFirst: "Select a group DM first",
        label: (c) => `${c.title || "(untitled)"} · ${c.id.slice(0, 8)}…`,
        title: (c) => c.id,
        selectedText: (c) => `${c.title || "(untitled)"} · ${c.id}`,
        selected: null,
      };

      // Who a new DM or group DM is with, picked by name from the workspace's
      // members. Built once the page has its elements.
      let dmPicker = null;

      let gdmPicker = null;

      function initDmPickers() {
        const notMe = () => [authorId()].filter(Boolean);
        dmPicker = memberPicker({ input: "dm-member-search", list: "dm-member-options", picked: "dm-picked", multiple: false, exclude: notMe });
        gdmPicker = memberPicker({ input: "gdm-member-search", list: "gdm-member-options", picked: "gdm-picked", multiple: true, exclude: notMe });
      }

      // A member picked in one workspace is not a member of the next.
      function clearDmPickers() {
        if (dmPicker) dmPicker.clear();
        if (gdmPicker) gdmPicker.clear();
      }


      function dmOther(c) {
        const me = authorId();
        return c.member_low_id === me ? c.member_high_id : c.member_low_id;
      }


      /** @param {ConversationKind} kind */
      async function loadConversations(kind) {
        const list = document.getElementById(kind.list);
        list.innerHTML = "";
        const me = authorId();
        if (!me) {
          renderState(list, "Sign in or set a valid bearer token");
          return;
        }
        if (kind.namesMembers) await loadMembers();
        const suffix = kind.listPath(me);
        const url = token() ? `${base()}${suffix}` : uiReadPath(suffix);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) {
            renderState(list, await responseError(res), "err");
            return;
          }
          const convos = await res.json();
          if (!convos.length) {
            renderState(list, kind.none);
            return;
          }
          convos.forEach((c) => {
            const li = document.createElement("li");
            const a = document.createElement("a");
            a.href = "#";
            a.textContent = kind.label(c);
            a.title = kind.title(c);
            a.onclick = (ev) => {
              ev.preventDefault();
              selectConversation(kind, c);
            };
            li.appendChild(a);
            list.appendChild(li);
          });
        } catch (e) {
          renderState(list, unreachable(e), "err");
        }
      }


      /** @param {ConversationKind} kind */
      function selectConversation(kind, c) {
        kind.selected = c;
        document.getElementById(kind.header).textContent = kind.selectedText(c);
        loadConversationMessages(c.thread_id, kind.messages);
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


      // After an open: the roster may have grown, the list has a new row, and
      // the conversation the server returned is the one shown.
      /** @param {ConversationKind} kind */
      async function conversationOpened(kind, opened) {
        await loadMembers();
        await loadConversations(kind);
        selectConversation(kind, opened);
      }


      async function openDm() {
        if (!requireAuthForWrite()) return;
        const me = authorId();
        if (!me) return showError("Sign in or set a valid bearer token");
        const other = dmPicker ? dmPicker.selected()[0] : "";
        if (!other) return showError("Choose who to message");
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
          if (dmPicker) dmPicker.clear();
          await conversationOpened(DM, opened);
        } catch (e) {
          showError(unreachable(e));
        }
      }


      async function openGroupDm() {
        if (!requireAuthForWrite()) return;
        const me = authorId();
        if (!me) return showError("Sign in or set a valid bearer token");
        const ids = gdmPicker ? gdmPicker.selected() : [];
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
          if (gdmPicker) gdmPicker.clear();
          document.getElementById("gdm-title").value = "";
          await conversationOpened(GROUP_DM, opened);
        } catch (e) {
          showError(unreachable(e));
        }
      }


      /** @param {ConversationKind} kind */
      async function sendConversationMessage(kind) {
        if (!requireAuthForWrite()) return;
        const me = authorId();
        if (!me) return showError("Sign in or set a valid bearer token");
        const c = kind.selected;
        if (!c) return showError(kind.selectFirst);
        const input = /** @type {HTMLInputElement} */ (document.getElementById(kind.body));
        const body = input.value.trim();
        if (!body) return showError("Message body required");
        try {
          const res = await writeApi(kind.sendButton, apiWritePath(kind.sendPath(c)), {
            method: "POST",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ body }),
          });
          if (!res.ok) {
            showError(await responseError(res));
            return;
          }
          input.value = "";
          await loadConversationMessages(c.thread_id, kind.messages);
        } catch (e) {
          showError(unreachable(e));
        }
      }

export { DM, GROUP_DM, clearDmPickers, dmOther, initDmPickers, loadConversationMessages, loadConversations, openDm, openGroupDm, selectConversation, sendConversationMessage };
