// @ts-check
import { base } from "./api.js";


      // Tell the user what to fix without stopping the page. The same message
      // again refreshes the one on screen rather than stacking a copy, and
      // says it again: a screen reader announces a change to an alert's
      // text, not a new timer, and a change undone in the same task can be
      // dropped, so the text is cleared now and put back in a later task.
      function showError(message) {
        const region = document.getElementById("toasts");
        let toast = [...region.children].find((t) => t.dataset.message === message);
        if (toast) {
          const text = toast.firstElementChild;
          clearTimeout(Number(toast.dataset.restore));
          text.textContent = "";
          toast.dataset.restore = String(setTimeout(() => (text.textContent = message), 100));
        } else {
          toast = document.createElement("div");
          toast.className = "toast";
          toast.setAttribute("role", "alert");
          toast.dataset.message = message;
          const text = document.createElement("span");
          text.textContent = message;
          const close = document.createElement("button");
          close.type = "button";
          close.setAttribute("aria-label", "Dismiss");
          close.textContent = "×";
          const shown = toast;
          close.onclick = () => dropToast(shown);
          toast.append(text, close);
          region.appendChild(toast);
          while (region.children.length > 3) dropToast(region.firstElementChild);
        }
        const shown = toast;
        clearTimeout(Number(toast.dataset.timer));
        toast.dataset.timer = String(setTimeout(() => dropToast(shown), 8000));
      }

      function dropToast(toast) {
        clearTimeout(Number(toast.dataset.timer));
        clearTimeout(Number(toast.dataset.restore));
        toast.remove();
      }

      function setStatus(msg, cls) {
        document.getElementById("status").textContent = msg;
        document.getElementById("status").className = cls || "";
      }

      function renderState(el, message, cls = "muted") {
        el.replaceChildren();
        const state = document.createElement(el.matches("ul, ol") ? "li" : "div");
        state.textContent = message;
        state.className = cls;
        el.appendChild(state);
      }

      function setLoading(el, message = "Loading…") {
        el.setAttribute("aria-busy", "true");
        renderState(el, message);
      }

      function clearLoading(el) {
        el.removeAttribute("aria-busy");
      }

      async function responseError(res, prefix = "") {
        let detail = "";
        try {
          const raw = await res.text();
          if (raw) {
            try {
              const problem = JSON.parse(raw);
              detail = problem.detail || problem.error || problem.message || problem.title || "";
            } catch (_e) {
              detail = raw;
            }
          }
        } catch (_e) {
          /* The status is still useful when the response body cannot be read. */
        }
        detail = String(detail).replace(/\s+/g, " ").trim().slice(0, 500);
        const lead = prefix ? `${prefix}: ` : "";
        const said = humanError(res.status, detail);
        // The person sees this sentence only. The server body and the status
        // code stay off the toast, the row, the thread, and the board.
        return `${lead}${said}`;
      }

      // Say what went wrong and what to do about it. The server body is read
      // only so a missing capability can be named. It is not shown, and neither
      // is the status code. A 409 is a rule refusing the action.
      function humanError(status, detail) {
        const needs = /(?:capability|needs?|requires?)[^a-z]*([a-z]+:[a-z_]+)/i.exec(detail || "");
        if (status === 401) return "Your token or session was not accepted. Use Change to set a working one";
        // Minting itself needs token:admin, so pointing at Tokens would loop.
        if (status === 403 && needs && needs[1] === "token:admin")
          return "Your token is not allowed to do this; it needs token:admin. Ask a workspace admin for a token (maidan init prints the first admin token)";
        if (status === 403)
          return needs
            ? `Your token is not allowed to do this; it needs ${needs[1]}. Mint a token with it in Tokens`
            : "Your token is not allowed to do this. Mint one with the right capability in Tokens";
        if (status === 404) return "Not found. It may have been deleted, or it belongs to another workspace";
        if (status === 409) return "Refused";
        if (status === 413) return "That is too large for the server to accept";
        if (status === 429) return "Too many requests. Wait a moment, then try again";
        if (status >= 500) return "The server hit an error. Try again; if it keeps failing, check the server log";
        return "The request was not accepted";
      }

      // A list row that acts on click is also a button for the keyboard: it
      // takes focus, and Enter or Space runs its onclick.
      function keyActivates(el, role = "button") {
        el.tabIndex = 0;
        // A list row keeps its listitem role (lists must hold list items);
        // anything else announces what it does.
        if (el.tagName !== "LI") el.setAttribute("role", role);
        el.addEventListener("keydown", (e) => {
          if (e.target !== el || (e.key !== "Enter" && e.key !== " ")) return;
          e.preventDefault();
          el.click();
        });
      }

      function setOut(obj) {
        document.getElementById("out").textContent =
          typeof obj === "string" ? obj : JSON.stringify(obj, null, 2);
      }

      let liveCount = 0;

      function appendLive(line) {
        const el = document.getElementById("live-feed");
        el.textContent += line + "\n";
        el.scrollTop = el.scrollHeight;
        liveCount += 1;
        document.getElementById("live-count").textContent = String(liveCount);
      }

      // The Live bar stays one slim row until the socket is up; the raw event
      // feed is opt-in (the board is the readable view of the same events).
      function setWsStatus(text, cls) {
        const el = document.getElementById("ws-status");
        el.textContent = text;
        el.className = cls || "";
        // A refusal or a reconnect countdown can be a sentence. It stays on
        // this line; the title has the rest, so it never becomes a banner.
        el.title = text || "";
        document.getElementById("live-panel").classList.toggle("connected", cls === "connected");
      }

      function toggleLiveFeed(show) {
        const feed = document.getElementById("live-feed");
        const btn = document.getElementById("live-toggle");
        const open = typeof show === "boolean" ? show : feed.hidden;
        feed.hidden = !open;
        btn.setAttribute("aria-expanded", String(open));
      }

      // A fetch that threw never reached the server (or the reply was not
      // JSON); say that in words, with the one thing to check.
      function unreachable(e) {
        return e instanceof SyntaxError
          ? `The server at ${base()} answered with something that is not JSON. Check that API base points at a Maidan server.`
          : `Could not reach the server at ${base()}. Check that it is running and that API base is right.`;
      }

export { appendLive, clearLoading, humanError, keyActivates, liveCount, renderState, responseError, setLoading, setOut, setStatus, setWsStatus, showError, toggleLiveFeed, unreachable };
