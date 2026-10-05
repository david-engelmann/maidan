// @ts-check
import { api, apiReadPath, apiWritePath, base, headers, pastedToken, persist, token, wid } from "./api.js";
import { loadChannels, setAttention } from "./board.js";
import { responseError, showError, unreachable } from "./feedback.js";
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

      // Trade a token for an HttpOnly session with its authority, and drop it
      // from the page. A server with no session key (404) cannot hold one, so
      // the token stays in this tab only, never in storage.
      async function exchangeToken(secret) {
        let res;
        try {
          res = await api(`${base()}/auth/session/from-token`, {
            method: "POST",
            headers: { Accept: "application/json", Authorization: "Bearer " + secret },
            credentials: "include",
          });
        } catch (e) {
          return { ok: false, error: unreachable(e) };
        }
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
        bearerMemberId = null;
        if (!token()) return null;
        try {
          const res = await api(`${base()}/me`, { headers: headers() });
          if (!res.ok) return null;
          const identity = await res.json();
          bearerMemberId = identity.member_id;
          if (!wid()) document.getElementById("workspace").value = identity.workspace_id;
          return identity;
        } catch (_e) {
          return null;
        }
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


      // The discovery document says which sign-in paths this server has, so the
      // first-run card never offers an identity provider that is not there.
      async function loadServerAuth() {
        oidcLoginPath = null;
        serverOffersSessions = null;
        try {
          const res = await api(`${base()}/.well-known/maidan.json`);
          if (res.ok) {
            const auth = (await res.json()).auth;
            if (auth && auth.oidc && auth.oidc_login) oidcLoginPath = auth.oidc_login;
            if (auth && typeof auth.sessions === "boolean") serverOffersSessions = auth.sessions;
          }
        } catch (_e) {
          /* no discovery document: a token is the only path offered */
        }
        document.getElementById("first-run-oidc").hidden = !oidcLoginPath;
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
        if (!signingIn) signingIn = trySignIn().finally(() => (signingIn = null));
        return signingIn;
      }

      async function trySignIn() {
        persist();
        const status = document.getElementById("session-status");
        const secret = pastedToken();
        if (!secret) {
          forgetBearerMember();
          showConnection(false);
          return;
        }
        const exchanged = await exchangeToken(secret);
        if (!exchanged.ok) {
          bearerMemberId = null;
          status.hidden = false;
          status.className = "err";
          status.textContent = exchanged.error;
          return;
        }
        await refreshSession();
        let res = null;
        try {
          res = await api(`${base()}/me`, { headers: headers() });
        } catch (e) {
          forgetBearerMember();
          status.hidden = false;
          status.className = "err";
          status.textContent = unreachable(e);
          return;
        }
        if (!res.ok) {
          forgetBearerMember();
          status.hidden = false;
          status.className = "err";
          status.textContent =
            res.status === 401
              ? "That token was not accepted: check it was copied whole and has not expired or been revoked, or mint a new one in Tokens."
              : await responseError(res, "Could not check that token");
          return;
        }
        const identity = await res.json();
        bearerMemberId = identity.member_id;
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
          res = await api(apiWritePath(`/workspaces/${wid()}`), {
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


      async function start() {
        setAttention(0);
        await loadServerAuth();
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
        await refreshBearerIdentity();
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

export { bearerMemberId, exchangeToken, forgetBearerMember, loadServerAuth, oidcLoginPath, refreshBearerIdentity, refreshSession, renderIdentity, saveWorkspaceName, serverOffersSessions, sessionMemberId, showConnection, showSecretOnce, signOutPostsLogout, start, tokenSession };
