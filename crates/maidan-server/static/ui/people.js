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
        el.style.setProperty("--hue", String(hueFor(id)));
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

      function authorId() {
        return token() ? bearerMemberId : sessionMemberId;
      }

export { ago, authorId, avatarEl, hueFor, initials, leaseLeft, loadMembers, memberKind, memberName, personEl };
