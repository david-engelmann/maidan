// @ts-check
import { base } from "./api.js";


      // The page's one feedback surface. `severity` says how loud a message
      // is: an "error" (the default) or a "warning" interrupts, as an alert;
      // a "success" says something worked, politely, and leaves sooner. The
      // region itself is a polite live region, so even a reader that misses
      // the alert role hears the line.
      //
      // The same message again refreshes the one on screen rather than
      // stacking a copy, and says it again: a screen reader announces a
      // change to an alert's text, not a new timer, and a change undone in
      // the same task can be dropped, so the text is cleared now and put back
      // in a later task.
      const SEVERITIES = ["error", "warning", "success"];

      function showError(message, severity = "error") {
        const level = SEVERITIES.includes(severity) ? severity : "error";
        const region = document.getElementById("toasts");
        let toast = [...region.children].find(
          (t) => t.dataset.message === message && t.dataset.severity === level,
        );
        if (toast) {
          const text = toast.firstElementChild;
          clearTimeout(Number(toast.dataset.restore));
          text.textContent = "";
          toast.dataset.restore = String(setTimeout(() => (text.textContent = message), 100));
        } else {
          toast = document.createElement("div");
          toast.className = `toast toast-${level}`;
          toast.setAttribute("role", level === "success" ? "status" : "alert");
          toast.dataset.message = message;
          toast.dataset.severity = level;
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
        toast.dataset.timer = String(setTimeout(() => dropToast(shown), level === "success" ? 5000 : 8000));
      }

      function dropToast(toast) {
        clearTimeout(Number(toast.dataset.timer));
        clearTimeout(Number(toast.dataset.restore));
        toast.remove();
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
          detail = problemDetail(await res.text());
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

      // Which credential the page acts with, from GET /me: "session" (a
      // sign-in, whose capabilities are fixed), "bearer" (a token, or the
      // session made from one), "delegated" (a token working as a member
      // under a delegation grant), or null until the page knows. An error
      // sentence names the credential the person actually holds.
      let identityMode = null;

      function setIdentityMode(mode) {
        identityMode = mode === "session" || mode === "bearer" || mode === "delegated" ? mode : null;
      }
export { setIdentityMode };

      // Say what went wrong and what to do about it. The server body is read
      // only so a missing capability can be named. It is not shown, and neither
      // is the status code. A 409 is a rule refusing the action.
      function humanError(status, detail, mode = identityMode) {
        const needs = /(?:capability|needs?|requires?)[^a-z]*([a-z]+:[a-z_]+)/i.exec(detail || "");
        if (status === 401) {
          // A session cannot be fixed with Change alone: it ended, so sign in.
          if (mode === "session") return "Your session was not accepted; it may have ended. Sign in again, or use Change to paste a token";
          if (mode === "delegated") return "This delegated token was not accepted; it or its grant may have ended. Use Change to set a working one";
          if (mode === "bearer") return "Your token was not accepted. Use Change to set a working one";
          return "Your token or session was not accepted. Use Change to set a working one";
        }
        if (status === 403) return refusal(needs ? needs[1] : null, mode);
        if (status === 404) return "Not found. It may have been deleted, or it belongs to another workspace";
        if (status === 409) return "Refused";
        if (status === 413) return "That is too large for the server to accept";
        if (status === 429) return "Too many requests. Wait a moment, then try again";
        if (status >= 500) return "The server hit an error. Try again; if it keeps failing, check the server log";
        return "The request was not accepted";
      }

      // A 403 in the words of the credential that was refused. Minting itself
      // needs token:admin, so pointing at Tokens would loop. A session cannot
      // mint at all, and a delegated token holds only what its grant lends.
      function refusal(cap, mode) {
        const who = mode === "session" ? "Your session" : mode === "delegated" ? "This delegated token" : "Your token";
        const lead = cap ? `${who} is not allowed to do this; it needs ${cap}.` : `${who} is not allowed to do this.`;
        if (cap === "token:admin")
          return `${lead} Ask a workspace admin for a token (maidan init prints the first admin token)`;
        // A pasted token, and the session made from one, is what a person
        // hands an agent, so it cannot accept a gate. Say the way that can.
        if (cap === "approval:grant")
          return `${lead} Accepting a gate needs you signed in through your identity provider, or a token an admin granted approval:grant. You can still decline or cancel it`;
        if (mode === "session")
          return cap
            ? `${lead} A session cannot mint tokens, so use Change to paste a token that has it`
            : `${lead} Use Change to paste a token with the right capability`;
        if (mode === "delegated")
          return cap
            ? `${lead} It holds only what its grant lends: ask for a grant that includes it`
            : `${lead} It holds only what its grant lends: ask for a grant with the right capability`;
        return cap ? `${lead} Mint a token with it in Tokens` : `${lead} Mint one with the right capability in Tokens`;
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

      // The part of an error reply worth reading for a capability name: the
      // problem's detail, or the raw text when it is not JSON.
      function problemDetail(raw) {
        if (!raw) return "";
        try {
          const problem = JSON.parse(raw);
          return problem.detail || problem.error || problem.message || problem.title || "";
        } catch (_e) {
          return raw;
        }
      }

      // The same sentence as responseError, for a reply whose body the caller
      // has already read (the raw output pane shows it).
      function textError(status, raw, prefix = "") {
        const detail = String(problemDetail(raw)).replace(/\s+/g, " ").trim().slice(0, 500);
        const lead = prefix ? `${prefix}: ` : "";
        return `${lead}${humanError(status, detail)}`;
      }

export { appendLive, clearLoading, humanError, keyActivates, liveCount, renderState, responseError, setLoading, setOut, setWsStatus, showError, textError, toggleLiveFeed, unreachable };
