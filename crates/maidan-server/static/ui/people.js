// @ts-check
import { api, base, headers, token, uiReadPath, wid } from "./api.js";
import { bearerMemberId, sessionMemberId } from "./session.js";
import { memberDirectory } from "./state.js";



      // ---- People -------------------------------------------------------
      function memberName(id) {
        if (!id) return "";
        const m = memberDirectory.get(id);
        return m ? m.name : `member ${String(id).slice(0, 8)}`;
      }

      function memberKind(id) {
        const m = memberDirectory.get(id);
        return m ? m.kind : "";
      }

      function initials(name) {
        const parts = String(name).replace(/[^\p{L}\p{N} ]+/gu, " ").trim().split(/\s+/);
        const letters = parts.length > 1 ? parts[0][0] + parts[1][0] : String(parts[0] || "?").slice(0, 2);
        return letters.toUpperCase();
      }

      function hueFor(id) {
        let h = 0;
        for (const c of String(id)) h = (h * 31 + c.charCodeAt(0)) >>> 0;
        return h % 360;
      }

      function avatarEl(id, large) {
        const el = document.createElement("span");
        const kind = memberKind(id) || "agent";
        el.className = `avatar ${kind}${large ? " lg" : ""}`;
        el.textContent = initials(memberName(id));
        el.classList.add("hue-" + (hueFor(id) % 12));
        el.setAttribute("aria-hidden", "true");
        return el;
      }

      // Avatar + display name (+ agent/human tag); the member id is the tooltip.
      function personEl(id, opts = {}) {
        const wrap = document.createElement("span");
        wrap.className = "person";
        wrap.title = id || "";
        if (opts.avatar !== false) wrap.appendChild(avatarEl(id));
        const name = document.createElement("span");
        name.className = "name";
        name.textContent = memberName(id);
        wrap.appendChild(name);
        const kind = memberKind(id);
        if (opts.tag && kind) {
          const tag = document.createElement("span");
          tag.className = "kind-tag";
          tag.textContent = kind;
          wrap.appendChild(tag);
        }
        return wrap;
      }

      function ago(ts) {
        if (!ts) return "";
        const secs = Math.max(0, Math.round((Date.now() - new Date(ts).getTime()) / 1000));
        if (secs < 45) return "just now";
        if (secs < 3600) return `${Math.round(secs / 60)}m ago`;
        if (secs < 86400) return `${Math.round(secs / 3600)}h ago`;
        return `${Math.round(secs / 86400)}d ago`;
      }

      function leaseLeft(ts) {
        if (!ts) return "";
        const secs = Math.round((new Date(ts).getTime() - Date.now()) / 1000);
        if (secs <= 0) return "lease lapsed";
        return secs < 120 ? `lease ${secs}s` : `lease ${Math.round(secs / 60)}m`;
      }

      // The workspace the directory was last filled from. A picker lists
      // members only when this is the workspace on screen, so a failed load
      // after a workspace change never offers the previous workspace's people.
      let membersWorkspace = "";

      async function loadMembers() {
        if (!wid()) return;
        const url =
          token() && !sessionMemberId
            ? `${base()}/workspaces/${wid()}/members`
            : uiReadPath(`/workspaces/${wid()}/members`);
        try {
          const res = await api(url, { headers: headers(), credentials: "include" });
          if (!res.ok) return;
          const rows = await res.json();
          memberDirectory.clear();
          membersWorkspace = wid();
          rows.forEach((m) => {
            memberDirectory.set(m.id, {
              name: m.display_name || m.handle,
              handle: m.handle,
              kind: m.kind,
            });
          });
        } catch (_e) {
          /* best-effort: names fall back to a short id */
        }
      }

      function membersLoadedFor() {
        return membersWorkspace;
      }

      // ---- Member picker ------------------------------------------------
      // A searchable list of the workspace's members, by display name or
      // handle, each marked agent or human, in the ARIA combobox pattern. A
      // person picks who they mean; nobody types an id. `exclude` returns the
      // ids not to offer (the signed-in person, and members already picked).
      function memberMatches(id, query) {
        const m = memberDirectory.get(id);
        if (!m) return false;
        if (!query) return true;
        const q = query.toLowerCase();
        return String(m.name).toLowerCase().includes(q) || String(m.handle || "").toLowerCase().includes(q);
      }

      function kindBadge(kind) {
        const badge = document.createElement("span");
        badge.className = `kind-badge ${kind === "human" ? "human" : "agent"}`;
        badge.textContent = kind === "human" ? "Human" : "Agent";
        return badge;
      }

      function memberPicker(opts) {
        const input = document.getElementById(opts.input);
        const list = document.getElementById(opts.list);
        const picked = document.getElementById(opts.picked);
        let chosen = [];
        let active = -1;
        let shown = [];
        let quiet = false;

        function close() {
          list.hidden = true;
          input.setAttribute("aria-expanded", "false");
          input.removeAttribute("aria-activedescendant");
          active = -1;
        }

        function highlight(index) {
          const options = list.querySelectorAll('[role="option"]');
          options.forEach((el, i) => el.setAttribute("aria-selected", i === index ? "true" : "false"));
          active = index;
          if (index >= 0 && options[index]) {
            input.setAttribute("aria-activedescendant", options[index].id);
            options[index].scrollIntoView({ block: "nearest" });
          } else {
            input.removeAttribute("aria-activedescendant");
          }
        }

        function render() {
          list.replaceChildren();
          const skip = new Set(opts.exclude().concat(chosen));
          const query = input.value.trim();
          const ready = membersLoadedFor() === wid();
          shown = ready
            ? Array.from(memberDirectory.keys())
                .filter((id) => !skip.has(id) && memberMatches(id, query))
                .sort((a, b) => memberName(a).localeCompare(memberName(b)))
            : [];
          if (!shown.length) {
            const li = document.createElement("li");
            li.className = "picker-empty";
            if (!ready) li.textContent = "Could not load this workspace's members";
            else if (query) li.textContent = `No member matches “${query}”`;
            else li.textContent = "No other members to add";
            list.appendChild(li);
          }
          shown.forEach((id, i) => {
            const li = document.createElement("li");
            li.id = `${opts.list}-${i}`;
            li.setAttribute("role", "option");
            li.setAttribute("aria-selected", "false");
            li.dataset.memberId = id;
            li.appendChild(personEl(id));
            const m = memberDirectory.get(id);
            if (m && m.handle) {
              const handle = document.createElement("span");
              handle.className = "handle";
              handle.textContent = `@${m.handle}`;
              li.appendChild(handle);
            }
            li.appendChild(kindBadge(memberKind(id)));
            // mousedown keeps focus in the search box; click does the pick.
            li.addEventListener("mousedown", (ev) => ev.preventDefault());
            li.addEventListener("click", () => choose(id));
            list.appendChild(li);
          });
          list.hidden = false;
          input.setAttribute("aria-expanded", "true");
          highlight(shown.length ? 0 : -1);
        }

        function renderPicked() {
          picked.replaceChildren();
          chosen.forEach((id) => {
            const chip = document.createElement("li");
            chip.className = "picked-member";
            chip.dataset.memberId = id;
            chip.appendChild(personEl(id));
            chip.appendChild(kindBadge(memberKind(id)));
            const remove = document.createElement("button");
            remove.type = "button";
            remove.className = "picked-remove";
            remove.textContent = "×";
            remove.setAttribute("aria-label", `Remove ${memberName(id)}`);
            remove.addEventListener("click", () => {
              chosen = chosen.filter((other) => other !== id);
              renderPicked();
              // Back to the search box, without dropping the list over the form.
              quiet = true;
              input.focus();
              quiet = false;
            });
            chip.appendChild(remove);
            picked.appendChild(chip);
          });
          if (opts.onChange) opts.onChange(chosen.slice());
        }

        // The list closes after each pick, so it never sits over the Title
        // field or the Open button; typing again reopens it.
        function choose(id) {
          chosen = opts.multiple ? chosen.concat([id]) : [id];
          input.value = "";
          renderPicked();
          close();
        }

        async function openList() {
          if (quiet) return;
          await loadMembers();
          if (document.activeElement === input) render();
        }

        input.addEventListener("focus", openList);
        input.addEventListener("click", () => {
          if (list.hidden) render();
        });
        input.addEventListener("input", render);
        input.addEventListener("blur", close);
        input.addEventListener("keydown", (ev) => {
          if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
            ev.preventDefault();
            if (list.hidden) {
              render();
              return;
            }
            if (!shown.length) return;
            const step = ev.key === "ArrowDown" ? 1 : -1;
            highlight((active + step + shown.length) % shown.length);
          } else if (ev.key === "Enter") {
            if (!list.hidden && active >= 0 && shown[active]) {
              ev.preventDefault();
              choose(shown[active]);
            }
          } else if (ev.key === "Escape") {
            close();
          } else if (ev.key === "Backspace" && !input.value && chosen.length) {
            chosen = chosen.slice(0, -1);
            renderPicked();
            render();
          }
        });

        function clearPicked() {
          chosen = [];
          input.value = "";
          renderPicked();
          close();
        }

        return { selected: () => chosen.slice(), clear: clearPicked };
      }

      function authorId() {
        return token() ? bearerMemberId : sessionMemberId;
      }

export { ago, authorId, avatarEl, hueFor, initials, kindBadge, leaseLeft, loadMembers, memberKind, memberMatches, memberName, memberPicker, membersLoadedFor, personEl };
