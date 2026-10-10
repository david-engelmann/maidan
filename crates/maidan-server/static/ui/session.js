// @ts-check
import { api, apiReadPath, apiWritePath, base, headers, pastedToken, persist, token, wid, writeApi } from "./api.js";
import { loadChannels, setAttention } from "./board.js";
import { responseError, setIdentityMode, showError, unreachable } from "./feedback.js";
import { loadNeedsYou } from "./needs.js";
import { authorId, personEl } from "./people.js";
import { connectWs } from "./realtime.js";
import { tokenKey, wsResumeKey } from "./state.js";



      let sessionMemberId = null;

      let oidcLoginPath = null;

      // `true` / `false` from discovery's `auth.sessions`. `null` means the
      // document never answered, which is not the same as "no sessions".
      let serverOffersSessions = null;

      // The session cookie was made from a pasted token (it holds that token's
      // authority), as opposed to an OIDC sign-in.
      let tokenSession = false;

      let bearerMemberId = null;

      // A /me check that throws never confirmed the secret just pasted.
      // Drop the member learned from the previous token so later reads
      // do not keep calling routes as that member.
      function forgetBearerMember() {
        bearerMemberId = null;
      }

      // Each token check takes a number. A check that a newer paste or a
      // newer read has overtaken must not set the member or the status line,
      // or a slow rejection of token A would sign out token B.
      let identityGen = 0;

      // Trade a token for an HttpOnly session with its authority, and drop it
      // from the page. A server with no session key (404) cannot hold one, so
      // the token stays in this tab only, never in storage. `current` says
      // whether the caller still wants the answer; an overtaken exchange
      // leaves the token field to the newer one.
      async function exchangeToken(secret, current = () => true) {
        let res;
        try {
          res = await writeApi(null, `${base()}/auth/session/from-token`, {
            method: "POST",
            headers: { Accept: "application/json", Authorization: "Bearer " + secret },
            credentials: "include",
          });
        } catch (e) {
          return { ok: false, error: unreachable(e) };
        }
        if (!current()) return { ok: false, stale: true };
        if (res.status === 404) {
          document.getElementById("token").value = secret;
          return { ok: true };
        }
        if (!res.ok) {
          document.getElementById("token").value = "";
          return {
            ok: false,
            error:
              res.status === 401
                ? "That token was not accepted: check it was copied whole and has not expired or been revoked, or mint a new one in Tokens."
                : await responseError(res, "Could not sign in with that token"),
          };
        }
        document.getElementById("token").value = "";
        tokenSession = true;
        return { ok: true };
      }

      async function refreshBearerIdentity() {
        const gen = ++identityGen;
        bearerMemberId = null;
        if (!token()) return null;
        try {
          const res = await api(`${base()}/me`, { headers: headers() });
          if (gen !== identityGen || !res.ok) return null;
          const identity = await res.json();
          if (gen !== identityGen) return null;
          bearerMemberId = identity.member_id;
          if (!wid()) document.getElementById("workspace").value = identity.workspace_id;
          return identity;
        } catch (_e) {
          if (gen === identityGen) bearerMemberId = null;
          return null;
        }
      }


      // What GET /me says the page acts with. A session made from a token
      // carries that token, so /me reports it as a bearer (or delegated) one.
      function credentialMode(me) {
        if (!me) return null;
        if (!me.is_bearer) return "session";
        return me.delegation_grant_id ? "delegated" : "bearer";
      }

      const MODE_WORDS = {
        session: ["session", "Signed-in session: pinned to this member, with a fixed set of capabilities"],
        bearer: ["bearer token", "Bearer token: acts as this member with the token's capabilities"],
        delegated: ["delegated token", "Delegated token: works as this member under a grant, with only what the grant lends"],
      };

      // The header says which credential is in use, and error sentences
      // name that credential rather than guessing.
      function showIdentityMode(mode) {
        setIdentityMode(mode);
        const el = document.getElementById("identity-mode");
        const words = mode ? MODE_WORDS[mode] : null;
        el.hidden = !words;
        el.textContent = words ? words[0] : "";
        el.title = words ? words[1] : "";
        el.dataset.mode = words ? mode : "";
      }

      async function refreshSession() {
        const el = document.getElementById("session-status");
        try {
          const res = await api(`${base()}/auth/session`, { credentials: "include" });
          if (!res.ok) {
            el.hidden = true;
            sessionMemberId = null;
            tokenSession = false;
            document.getElementById("mint").hidden = true;
            return null;
          }
          const body = await res.json();
          sessionMemberId = body.member_id;
          tokenSession = Boolean(body.token_id);
          el.hidden = false;
          const shown = typeof body.display_name === "string" ? body.display_name.trim() : "";
          el.textContent = shown ? `Signed in · ${shown}` : `Signed in · ${body.member_id}`;
          el.title = body.member_id;
          el.className = "ok";
          if (!wid()) document.getElementById("workspace").value = body.workspace_id;
          if (!document.getElementById("token-member").value) {
            document.getElementById("token-member").value = body.member_id;
          }
          document.getElementById("mint").hidden = tokenSession;
          return body;
        } catch (e) {
          el.textContent = String(e);
          el.className = "err";
          sessionMemberId = null;
          return null;
        }
      }


      // Discovery is optional, so it is bounded and never holds up a
      // credential. A newer read (the API base changed) wins over an older
      // one still in flight, and a read that never got an answer is tried
      // again, so one dropped request does not hide the identity provider
      // until the page is reloaded.
      const DISCOVERY_TIMEOUT_MS = 5000;
      const DISCOVERY_RETRY_MS = [2000, 5000, 15000];
      let discoveryGen = 0;
      let discoveryRetry = null;

      // The discovery document says which sign-in paths this server has, so the
      // first-run card never offers an identity provider that is not there.
      function loadServerAuth() {
        return discoverServerAuth(0);
      }

      async function discoverServerAuth(attempt) {
        const gen = ++discoveryGen;
        clearTimeout(discoveryRetry);
        oidcLoginPath = null;
        serverOffersSessions = null;
        document.getElementById("first-run-oidc").hidden = true;
        let answered = false;
        let loginPath = null;
        let sessions = null;
        try {
          const res = await api(`${base()}/.well-known/maidan.json`, {
            signal: AbortSignal.timeout(DISCOVERY_TIMEOUT_MS),
          });
          if (res.ok) {
            const auth = (await res.json()).auth;
            if (auth && auth.oidc && auth.oidc_login) loginPath = auth.oidc_login;
            if (auth && typeof auth.sessions === "boolean") sessions = auth.sessions;
            answered = true;
          } else if (res.status < 500 && res.status !== 429) {
            // No discovery document: a token is the only path offered.
            answered = true;
          }
        } catch (_e) {
          /* unreachable, timed out, or not JSON: try again below */
        }
        if (gen !== discoveryGen) return;
        oidcLoginPath = loginPath;
        serverOffersSessions = sessions;
        document.getElementById("first-run-oidc").hidden = !oidcLoginPath;
        if (!answered && attempt < DISCOVERY_RETRY_MS.length) {
          discoveryRetry = setTimeout(() => discoverServerAuth(attempt + 1), DISCOVERY_RETRY_MS[attempt]);
        }
      }

      // A missing cached member is not proof the cookie is gone: the session
      // read can fail while the cookie is still live. Post logout when the
      // server said it offers sessions, or when discovery never answered.
      // A server that said it has none has no cookie to end, unless this page
      // has already seen a member.
      function signOutPostsLogout(sessions, cachedMemberId) {
        if (sessions === true) return true;
        if (sessions === false) return Boolean(cachedMemberId);
        return true;
      }

      // Signing out ends the session on the server, whichever kind it is; the
      // token it was made from stays valid until revoked. A token kept only in
      // this tab (no session) is forgotten here.
      document.getElementById("logout").onclick = () => {
        localStorage.removeItem(tokenKey);
        localStorage.removeItem(wsResumeKey);
        document.getElementById("token").value = "";
        tokenSession = false;
        if (!signOutPostsLogout(serverOffersSessions, sessionMemberId)) {
          window.location.reload();
          return;
        }
        const form = document.createElement("form");
        form.method = "POST";
        form.action = `${base()}/auth/logout`;
        document.body.appendChild(form);
        form.submit();
      };

      // A secret is shown once, where the page shows every new secret.
      function showSecretOnce(title, secret) {
        document.getElementById("mint-title").textContent = title;
        document.getElementById("mint-secret").textContent = secret;
        const banner = document.getElementById("mint-banner");
        banner.classList.add("on");
        banner.scrollIntoView({ block: "nearest" });
        document.getElementById("copy-secret").focus();
      }


      // Pasting a token signs in: exchange it for a session, check it against
      // /me, then load the channels and what waits on you, or say in words why
      // it was not accepted. Enter, Sign in and leaving the field all land
      // here; a second trigger while one is running joins it rather than
      // spending the token twice.
      let signingIn = null;

      function signInWithToken() {
        if (!signingIn)
          signingIn = trySignIn().finally(() => {
            signingIn = null;
            document.getElementById("token").disabled = false;
            document.getElementById("token-signin").disabled = false;
          });
        return signingIn;
      }

      async function trySignIn() {
        document.getElementById("token").disabled = true;
        document.getElementById("token-signin").disabled = true;
        persist();
        const gen = ++identityGen;
        const current = () => gen === identityGen;
        const status = document.getElementById("session-status");
        const secret = pastedToken();
        if (!secret) {
          forgetBearerMember();
          showConnection(false);
          return;
        }
        const exchanged = await exchangeToken(secret, current);
        if (!current()) return;
        if (!exchanged.ok) {
          bearerMemberId = null;
          showIdentityMode(null);
          status.hidden = false;
          status.className = "err";
          status.textContent = exchanged.error;
          return;
        }
        await refreshSession();
        loadWorkspaceSwitcher();
        if (!current()) return;
        let res = null;
        try {
          res = await api(`${base()}/me`, { headers: headers() });
        } catch (e) {
          if (current()) {
            forgetBearerMember();
            showIdentityMode(null);
            status.hidden = false;
            status.className = "err";
            status.textContent = unreachable(e);
          }
          return;
        }
        if (!current()) return;
        if (!res.ok) {
          const said =
            res.status === 401
              ? "That token was not accepted: check it was copied whole and has not expired or been revoked, or mint a new one in Tokens."
              : await responseError(res, "Could not check that token");
          if (!current()) return;
          forgetBearerMember();
          showIdentityMode(null);
          status.hidden = false;
          status.className = "err";
          status.textContent = said;
          return;
        }
        const identity = await res.json();
        if (!current()) return;
        bearerMemberId = identity.member_id;
        showIdentityMode(credentialMode(identity));
        if (!wid()) document.getElementById("workspace").value = identity.workspace_id;
        persist();
        // A later success must not leave the previous rejection on screen.
        status.hidden = true;
        status.textContent = "";
        status.className = "muted";
        await loadChannels();
        await renderIdentity(identity.member_id);
        showConnection(false);
        loadNeedsYou();
      }

      document.getElementById("token").addEventListener("change", () => signInWithToken());

      document.getElementById("token").addEventListener("keydown", (e) => {
        if (e.key !== "Enter") return;
        if (e.isComposing) return;
        e.preventDefault();
        signInWithToken();
      });

      document.getElementById("token-signin").addEventListener("click", () => {
        // Leaving the field for the button already started the sign-in.
        if (signingIn) return;
        if (!pastedToken()) {
          if (authorId()) return;
          document.getElementById("token-hint").textContent = "Paste your token to connect.";
          document.getElementById("token").focus();
          return;
        }
        signInWithToken();
      });


      // Sets the workspace display name. The id in the workspace field stays.
      async function saveWorkspaceName() {
        const name = document.getElementById("workspace-name").value.trim();
        if (!wid()) {
          showError("Set a workspace id before naming it.");
          return;
        }
        if (!name) {
          showError("A workspace name cannot be empty.");
          return;
        }
        let res;
        try {
          res = await writeApi("workspace-name-save", apiWritePath(`/workspaces/${wid()}`), {
            method: "PATCH",
            headers: headers(true),
            credentials: "include",
            body: JSON.stringify({ name }),
          });
        } catch (e) {
          showError(unreachable(e));
          return;
        }
        if (!res.ok) {
          showError(await responseError(res, "Could not name the workspace"));
          return;
        }
        const ws = await res.json();
        const saved = ws.name || name;
        document.getElementById("identity-ws").textContent = saved;
        document.getElementById("workspace-name").value = saved;
      }


      // Connection chrome. Until this browser holds a working credential (a
      // stored token or a signed-in session) the three inputs sit in the
      // board's first-run card; after, the header shows the name and the
      // workspace name, and "Change" opens the inputs. The inputs move
      // rather than being copied, so there is one value of each.
      function showConnection(editing) {
        const connected = !!authorId();
        const fields = document.getElementById("conn-fields");
        const pill = document.getElementById("identity-pill");
        if (connected) pill.after(fields);
        else document.getElementById("first-run-fields").append(fields);
        document.getElementById("first-run").hidden = connected;
        const emptyConnect = document.querySelector("#board-onboard [data-open-connect]");
        if (emptyConnect) emptyConnect.className = connected ? "primary" : "ghost";
        fields.hidden = connected && !editing;
        pill.hidden = !connected || editing;
        document.getElementById("logout").hidden = !connected;
        const canName = connected;
        document.getElementById("workspace-name-label").hidden = !canName;
        document.getElementById("workspace-name-save").hidden = !canName;
      }

      async function renderIdentity(memberId) {
        const who = document.getElementById("identity-who");
        who.replaceChildren(personEl(memberId, { avatar: false }));
        const wsLabel = document.getElementById("identity-ws");
        wsLabel.textContent = `workspace ${wid().slice(0, 8)}…`;
        if (wid()) {
          try {
            const res = await api(apiReadPath(`/workspaces/${wid()}`), {
              headers: headers(),
              credentials: "include",
            });
            if (res.ok) {
              const ws = await res.json();
              if (ws.name) {
                wsLabel.textContent = ws.name;
                const nameInput = document.getElementById("workspace-name");
                if (document.activeElement !== nameInput) nameInput.value = ws.name;
              }
            }
          } catch (_e) {
            /* the short id stays */
          }
        }
        const status = document.getElementById("session-status");
        if (token() && !sessionMemberId) status.hidden = true;
      }


      // The workspace switcher (Open Work Next 6, docs/Hosted Console.md). A
      // person signed in with the identity provider who is a member of more
      // than one workspace can switch. The server lists only the workspaces of
      // the identity this session signed in with; switching is a fresh sign-in
      // to the chosen workspace, never a session minted from this one.
      /** @type {{workspace_id: string, name: string, member_id: string, handle: string, current: boolean}[]} */
      let switchable = [];
      // A newer load or hide wins over an older load still in flight, so a
      // late answer for an earlier session never reveals the switcher.
      let switcherGen = 0;

      function hideWorkspaceSwitcher() {
        switcherGen += 1;
        switchable = [];
        document.getElementById("ws-switch").hidden = true;
        closeWorkspaceSwitcher();
      }

      async function loadWorkspaceSwitcher() {
        if (!sessionMemberId || tokenSession) return hideWorkspaceSwitcher();
        const gen = ++switcherGen;
        let listed = [];
        try {
          const res = await api(`${base()}/auth/session/workspaces`, { credentials: "include" });
          if (gen !== switcherGen) return;
          if (!res.ok) return hideWorkspaceSwitcher();
          const body = await res.json();
          listed = Array.isArray(body.workspaces) ? body.workspaces : [];
        } catch (_e) {
          if (gen === switcherGen) hideWorkspaceSwitcher();
          return;
        }
        if (gen !== switcherGen) return;
        // The session may have changed while the list was on its way.
        if (!sessionMemberId || tokenSession) return hideWorkspaceSwitcher();
        switchable = listed;
        document.getElementById("ws-switch").hidden = switchable.length < 2;
      }

      function renderWorkspaceSwitcher() {
        const query = /** @type {HTMLInputElement} */ (document.getElementById("ws-switch-search")).value
          .trim()
          .toLowerCase();
        const list = document.getElementById("ws-switch-list");
        const shown = switchable.filter((w) => !query || w.name.toLowerCase().includes(query));
        list.replaceChildren(
          ...shown.map((w) => {
            const li = document.createElement("li");
            li.dataset.id = w.workspace_id;
            if (w.current) li.setAttribute("aria-current", "true");
            const button = document.createElement("button");
            button.type = "button";
            button.className = "ghost";
            button.textContent = w.current ? `${w.name} (current)` : w.name;
            button.title = `Signed in there as ${w.handle}`;
            button.disabled = w.current;
            button.onclick = () => switchWorkspace(w.workspace_id);
            li.append(button);
            return li;
          }),
        );
        if (!shown.length) {
          const li = document.createElement("li");
          li.className = "muted";
          li.textContent = "No workspace matches.";
          list.append(li);
        }
      }

      function openWorkspaceSwitcher() {
        const panel = document.getElementById("ws-switcher");
        const search = /** @type {HTMLInputElement} */ (document.getElementById("ws-switch-search"));
        search.value = "";
        renderWorkspaceSwitcher();
        panel.hidden = false;
        document.getElementById("ws-switch").setAttribute("aria-expanded", "true");
        search.focus();
      }

      function closeWorkspaceSwitcher() {
        document.getElementById("ws-switcher").hidden = true;
        document.getElementById("ws-switch").setAttribute("aria-expanded", "false");
      }

      function switchWorkspace(workspaceId) {
        const target = switchable.find((w) => w.workspace_id === workspaceId && !w.current);
        if (!target) return;
        /** @type {HTMLInputElement} */ (document.getElementById("workspace")).value = target.workspace_id;
        persist();
        const login = oidcLoginPath || "/auth/oidc/login";
        window.location.href =
          `${base()}${login}?workspace_id=${encodeURIComponent(target.workspace_id)}&return_to=/ui/`;
      }

      function initWorkspaceSwitcher() {
        document.getElementById("ws-switch").onclick = () => {
          if (document.getElementById("ws-switcher").hidden) openWorkspaceSwitcher();
          else closeWorkspaceSwitcher();
        };
        const search = document.getElementById("ws-switch-search");
        search.addEventListener("input", renderWorkspaceSwitcher);
        search.addEventListener("keydown", (e) => {
          if (e.key === "Escape") {
            closeWorkspaceSwitcher();
            document.getElementById("ws-switch").focus();
          } else if (e.key === "Enter") {
            const first = Array.from(document.querySelectorAll("#ws-switch-list li button")).find(
              (b) => b instanceof HTMLButtonElement && !b.disabled,
            );
            if (first instanceof HTMLButtonElement) first.click();
          }
        });
      }

      async function start() {
        setAttention(0);
        loadServerAuth();
        // An older page kept the token in localStorage: exchange it once and
        // remove it.
        const stored = localStorage.getItem(tokenKey);
        let storedTokenError = null;
        if (stored) {
          localStorage.removeItem(tokenKey);
          const exchanged = await exchangeToken(stored);
          if (!exchanged.ok) {
            storedTokenError =
              "The token saved in this browser did not sign in: it may have expired or been revoked. Paste a new one, or sign in another way.";
          }
        }
        await refreshSession();
        loadWorkspaceSwitcher();
        const identity = await refreshBearerIdentity();
        // A token (pasted, or the session made from one) answered /me; with
        // none, a session alone is a sign-in.
        showIdentityMode(identity ? credentialMode(identity) : sessionMemberId ? "session" : null);
        // refreshSession hides the status when there is no session, which
        // would swallow the failure of the token an older page had stored.
        if (storedTokenError) {
          const status = document.getElementById("session-status");
          status.hidden = false;
          status.className = "err";
          status.textContent = storedTokenError;
        }
        await loadChannels();
        const me = authorId();
        if (me) {
          await renderIdentity(me);
          connectWs();
          loadNeedsYou();
        } else if (token()) {
          const status = document.getElementById("session-status");
          status.hidden = false;
          status.className = "err";
          status.textContent =
            "The token saved in this browser did not sign in: it may have expired or been revoked. Paste a new one, or sign in another way.";
        }
        showConnection(false);
      }

export { initWorkspaceSwitcher, loadWorkspaceSwitcher, bearerMemberId, credentialMode, exchangeToken, forgetBearerMember, loadServerAuth, oidcLoginPath, refreshBearerIdentity, refreshSession, renderIdentity, saveWorkspaceName, serverOffersSessions, sessionMemberId, showConnection, showIdentityMode, showSecretOnce, signOutPostsLogout, start, tokenSession };
