// @ts-check
import { OPERATIONS } from "./client.js";
import { showError } from "./feedback.js";
import { oidcLoginPath, sessionMemberId, tokenSession } from "./session.js";
import { baseInput, tokenKey, wsKey } from "./state.js";

/**
 * One fetch for the board. Callers pass the same options as before
 * (method, headers, credentials, body). The production build does not bundle this.
 * @param {string} url
 * @param {RequestInit=} options
 * @returns {Promise<Response>}
 */
      async function api(url, options) {
        return fetch(url, options);
      }



      function base() {
        return baseInput.value.replace(/\/$/, "");
      }

      function wsUrl() {
        return base().replace(/^http/, "ws").replace(/^https/, "wss") + "/ws/subscribe";
      }

      function wid() {
        return document.getElementById("workspace").value.trim();
      }

      // A token in the field is there only until the server has exchanged it
      // for a session cookie that no script on the page can read.
      function pastedToken() {
        return document.getElementById("token").value.trim();
      }

      // Whether the page acts with a token's authority: one just pasted, or the
      // session made from it.
      function token() {
        return Boolean(pastedToken()) || tokenSession;
      }

      function persist() {
        localStorage.removeItem(tokenKey);
        localStorage.setItem(wsKey, wid());
      }

      function headers(json) {
        const h = { Accept: "application/json" };
        if (json) h["Content-Type"] = "application/json";
        const t = pastedToken();
        if (t) h["Authorization"] = "Bearer " + t;
        return h;
      }

      function uiReadPath(suffix) {
        return `${base()}/ui/api${suffix}`;
      }

      // A read the bearer API and the session proxy both serve. Token
      // authority goes to the bearer route; a signed-in session goes through
      // /ui/api. Same split as a write.
      function apiReadPath(suffix) {
        return token() ? `${base()}${suffix}` : uiReadPath(suffix);
      }

      // Writes go straight to the bearer API when a token is set, else through
      // the session-authed /ui/api proxy (same suffix).
      function apiWritePath(suffix) {
        return token() ? `${base()}${suffix}` : uiReadPath(suffix);
      }

      // Purge, peers, mint and revoke stay on the bearer tree. A session is
      // told so in a sentence and is not sent there to read a raw error.
      function requireBearer() {
        if (token()) return true;
        showError(
          sessionMemberId
            ? "This needs a bearer token. A signed-in session cannot call it."
            : "Set a bearer token to do this."
        );
        return false;
      }

      // A write is allowed with either a bearer token or a signed-in session.
      // Without either the board shows Connect this browser, so the sentence
      // points there, and names the identity provider only when there is one.
      function requireAuthForWrite() {
        if (token() || sessionMemberId) return true;
        showError(
          oidcLoginPath
            ? "Sign in first: paste a token under Connect this browser, or sign in with your identity provider."
            : "Sign in first: paste a token under Connect this browser."
        );
        return false;
      }

      // A mutating request from a board button. The button disables while the
      // request is in flight, so a double click cannot fire twice, and the
      // request carries a fresh Idempotency-Key, so a duplicated delivery
      // of the same request replays the first response instead of writing
      // again. Each call is one request with one key: a later call is a new
      // write, not a retry of this one. `button` is an element, an element
      // id, a list of elements, or null when no button owns the call.
      // @param {HTMLElement|string|NodeList|Array|null} button
      // @param {string} url
      // @param {RequestInit=} options
      // @returns {Promise<Response>}
      function writeApi(button, url, options) {
        const els =
          typeof button === "string"
            ? [document.getElementById(button)]
            : button && typeof button.forEach === "function"
              ? Array.from(button)
              : [button];
        // Only real buttons disable; anything else still gets the key.
        const targets =
          typeof HTMLButtonElement === "undefined"
            ? []
            : els.filter((el) => el instanceof HTMLButtonElement);
        targets.forEach((el) => {
          el.disabled = true;
        });
        const key =
          typeof crypto !== "undefined" && crypto.randomUUID
            ? crypto.randomUUID()
            : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
        const init = { ...(options || {}) };
        init.headers = { ...(init.headers || {}), "Idempotency-Key": key };
        return api(url, init).finally(() => {
          targets.forEach((el) => {
            el.disabled = false;
          });
        });
      }

export { OPERATIONS, api, apiReadPath, apiWritePath, base, headers, pastedToken, persist, requireAuthForWrite, requireBearer, token, uiReadPath, wid, writeApi, wsUrl };
