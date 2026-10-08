// Unit tests for the page helpers the module split left without a spec.
// Node has no document, so the stub is installed before the modules load.
// The modules are the ones the board serves. Nothing here is a copy of them.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, test } from "node:test";

function makeEl() {
  const children = [];
  return {
    value: "",
    hidden: false,
    className: "",
    textContent: "",
    innerHTML: "",
    title: "",
    tabIndex: 0,
    open: false,
    method: "",
    action: "",
    dataset: {},
    style: {},
    children,
    classList: {
      toggle() {},
      add() {},
      remove() {},
      contains() {
        return false;
      },
    },
    setAttribute() {},
    getAttribute() {
      return null;
    },
    removeAttribute() {},
    addEventListener() {},
    removeEventListener() {},
    append(...nodes) {
      children.push(...nodes);
    },
    after() {},
    prepend() {},
    appendChild(child) {
      children.push(child);
      return child;
    },
    replaceChildren() {
      children.length = 0;
    },
    remove() {},
    click() {},
    close() {},
    showModal() {},
    focus() {},
    submit() {},
    scrollIntoView() {},
    querySelector() {
      return makeEl();
    },
    querySelectorAll() {
      return [];
    },
    matches() {
      return false;
    },
    get firstElementChild() {
      return children[0] || null;
    },
  };
}

const byId = new Map();
globalThis.document = {
  getElementById(id) {
    if (!byId.has(id)) byId.set(id, makeEl());
    return byId.get(id);
  },
  querySelectorAll() {
    return [];
  },
  querySelector() {
    return makeEl();
  },
  addEventListener() {},
  createElement(tag) {
    const el = makeEl();
    el.tagName = String(tag).toUpperCase();
    return el;
  },
  body: makeEl(),
};
globalThis.window = {
  location: { origin: "http://127.0.0.1:8080", reload() {} },
  matchMedia() {
    return { matches: false, addEventListener() {}, removeEventListener() {} };
  },
  addEventListener() {},
};
globalThis.localStorage = {
  store: new Map(),
  getItem(key) {
    return this.store.has(key) ? this.store.get(key) : null;
  },
  setItem(key, value) {
    this.store.set(key, String(value));
  },
  removeItem(key) {
    this.store.delete(key);
  },
};
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: { platform: "Linux", userAgent: "node" },
});

const { apiReadPath, apiWritePath, requireBearer } = await import("./api.js");
const { humanError, showError } = await import("./feedback.js");
const { setIdentityMode } = await import("./feedback.js");
const { trimImageCache } = await import("./artifacts.js");
const { registerBrowserPush } = await import("./push.js");
const session = await import("./session.js");
const { credentialMode, loadServerAuth, refreshSession, signOutPostsLogout } = session;

const BASE = "http://127.0.0.1:8080";

function toastMessages() {
  return document.getElementById("toasts").children.map((toast) => toast.dataset.message);
}

function clearToasts() {
  const region = document.getElementById("toasts");
  for (const toast of region.children) {
    clearTimeout(Number(toast.dataset.timer));
    clearTimeout(Number(toast.dataset.restore));
  }
  region.children.length = 0;
}

function jsonResponse(body, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
    text: async () => JSON.stringify(body),
  };
}

// Lets the awaits inside a page function run to the end.
function settle() {
  return new Promise((resolve) => setImmediate(resolve));
}

// A failed session read is how the page itself clears a signed-in session.
async function resetAuth() {
  document.getElementById("base").value = BASE;
  document.getElementById("token").value = "";
  clearToasts();
  globalThis.fetch = async () => ({
    ok: false,
    status: 401,
    json: async () => ({}),
    text: async () => "",
  });
  await refreshSession();
}

async function signIn(body) {
  globalThis.fetch = async (url) => {
    if (!String(url).endsWith("/auth/session")) throw new Error(`unexpected fetch ${url}`);
    return {
      ok: true,
      status: 200,
      json: async () => body,
      text: async () => JSON.stringify(body),
    };
  };
  return refreshSession();
}

describe("board page helpers", { concurrency: 1 }, () => {
  test("a session read goes through /ui/api and a pasted token goes to the bearer API", async () => {
    await resetAuth();
    document.getElementById("base").value = `${BASE}/`;
    assert.equal(apiReadPath("/threads/t1"), `${BASE}/ui/api/threads/t1`);
    assert.equal(apiWritePath("/threads/t1"), `${BASE}/ui/api/threads/t1`);

    document.getElementById("token").value = "  ";
    assert.equal(apiReadPath("/me"), `${BASE}/ui/api/me`);
    assert.equal(apiWritePath("/me"), `${BASE}/ui/api/me`);

    document.getElementById("token").value = "sekrit";
    assert.equal(apiReadPath("/threads/t1"), `${BASE}/threads/t1`);
    assert.equal(apiWritePath("/messages/m1"), `${BASE}/messages/m1`);
  });

  test("a session made from a token uses the bearer path with the field empty", async () => {
    await resetAuth();
    document.getElementById("token").value = "";
    const body = await signIn({
      member_id: "mem_1",
      token_id: "tok_1",
      display_name: "Ada",
      workspace_id: "ws_1",
    });
    assert.equal(body.member_id, "mem_1");
    assert.equal(apiReadPath("/workspaces/ws_1"), `${BASE}/workspaces/ws_1`);
    assert.equal(apiWritePath("/workspaces/ws_1/channels"), `${BASE}/workspaces/ws_1/channels`);
  });

  test("requireBearer allows a token and tells a session it cannot call that route", async () => {
    await resetAuth();
    assert.equal(requireBearer(), false);
    assert.deepEqual(toastMessages(), ["Set a bearer token to do this."]);

    clearToasts();
    document.getElementById("token").value = "sekrit";
    assert.equal(requireBearer(), true);
    assert.deepEqual(toastMessages(), []);

    document.getElementById("token").value = "";
    await signIn({
      member_id: "mem_1",
      token_id: null,
      display_name: "Ada",
      workspace_id: "ws_1",
    });
    clearToasts();
    assert.equal(requireBearer(), false);
    assert.deepEqual(toastMessages(), [
      "This needs a bearer token. A signed-in session cannot call it.",
    ]);
  });

  test("a write with no credential points at Connect this browser, not at a header token", async () => {
    const { requireAuthForWrite } = await import("./api.js");
    const { loadServerAuth } = await import("./session.js");
    await resetAuth();
    assert.equal(requireAuthForWrite(), false);
    assert.deepEqual(toastMessages(), ["Sign in first: paste a token under Connect this browser."]);

    clearToasts();
    globalThis.fetch = async () => ({
      ok: true,
      status: 200,
      json: async () => ({ auth: { bearer: true, oidc: true, oidc_login: "/auth/oidc/login", sessions: true } }),
      text: async () => "",
    });
    await loadServerAuth();
    assert.equal(requireAuthForWrite(), false);
    assert.deepEqual(toastMessages(), [
      "Sign in first: paste a token under Connect this browser, or sign in with your identity provider.",
    ]);
    clearToasts();
    document.getElementById("token").value = "sekrit";
    assert.equal(requireAuthForWrite(), true);
    await resetAuth();
  });

  test("sign out posts logout when the server offers sessions, even with no cached member", () => {
    assert.equal(signOutPostsLogout(true, null), true);
    assert.equal(signOutPostsLogout(true, ""), true);
    assert.equal(signOutPostsLogout(true, "mem_1"), true);
    assert.equal(signOutPostsLogout(false, null), false);
    assert.equal(signOutPostsLogout(false, ""), false);
    assert.equal(signOutPostsLogout(false, "mem_1"), true);
    // Discovery never answered: a missing cache is not proof there is no cookie.
    assert.equal(signOutPostsLogout(null, null), true);
    assert.equal(signOutPostsLogout(undefined, null), true);
  });

  test("humanError names the capability and never the status or the raw body", () => {
    assert.equal(
      humanError(401, "unauthorized"),
      "Your token or session was not accepted. Use Change to set a working one",
    );
    assert.equal(
      humanError(403, "requires token:admin"),
      "Your token is not allowed to do this; it needs token:admin. Ask a workspace admin for a token (maidan init prints the first admin token)",
    );
    assert.equal(
      humanError(403, "needs capability thread:write"),
      "Your token is not allowed to do this; it needs thread:write. Mint a token with it in Tokens",
    );
    assert.equal(
      humanError(403, "forbidden"),
      "Your token is not allowed to do this. Mint one with the right capability in Tokens",
    );
    assert.equal(
      humanError(404, "no such thread"),
      "Not found. It may have been deleted, or it belongs to another workspace",
    );
    assert.equal(humanError(409, "already closed"), "Refused");
    assert.equal(humanError(413, "too big"), "That is too large for the server to accept");
    assert.equal(humanError(429, "slow down"), "Too many requests. Wait a moment, then try again");
    assert.equal(
      humanError(500, "stack trace"),
      "The server hit an error. Try again; if it keeps failing, check the server log",
    );
    assert.equal(
      humanError(503, ""),
      "The server hit an error. Try again; if it keeps failing, check the server log",
    );
    assert.equal(humanError(400, "bad request detail"), "The request was not accepted");
    for (const status of [400, 401, 403, 404, 409, 413, 429, 500]) {
      const said = humanError(status, "raw server body");
      assert.equal(said.includes("raw server body"), false);
      assert.equal(said.includes(String(status)), false);
    }
  });

  test("credentialMode reads session, bearer, or delegated from /me", () => {
    assert.equal(credentialMode(null), null);
    assert.equal(credentialMode({ is_bearer: false, delegation_grant_id: null }), "session");
    assert.equal(credentialMode({ is_bearer: true, delegation_grant_id: null }), "bearer");
    assert.equal(credentialMode({ is_bearer: true, delegation_grant_id: "grant_1" }), "delegated");
  });

  test("humanError names the credential in use and never says token to a session", () => {
    const refusals = [
      [401, "unauthorized"],
      [403, "requires token:admin"],
      [403, "needs capability thread:write"],
      [403, "forbidden"],
      [403, "This needs a bearer token. A signed-in session cannot call it."],
    ];
    for (const [status, detail] of refusals) {
      const said = humanError(status, detail, "session");
      assert.match(said, /^Your session /, said);
      assert.doesNotMatch(said, /\byour token\b|\btoken or session\b|in Tokens/i, said);
      assert.doesNotMatch(humanError(status, detail, "delegated"), /^Your token/, detail);
      assert.match(humanError(status, detail, "delegated"), /^This delegated token /, detail);
      assert.match(humanError(status, detail, "bearer"), /^Your token /, detail);
    }
    assert.equal(
      humanError(401, "", "session"),
      "Your session was not accepted; it may have ended. Sign in again, or use Change to paste a token",
    );
    assert.equal(
      humanError(403, "needs capability thread:write", "session"),
      "Your session is not allowed to do this; it needs thread:write. A session cannot mint tokens, so use Change to paste a token that has it",
    );
    assert.equal(
      humanError(403, "needs capability thread:write", "delegated"),
      "This delegated token is not allowed to do this; it needs thread:write. It holds only what its grant lends: ask for a grant that includes it",
    );
    assert.equal(humanError(401, "", "bearer"), "Your token was not accepted. Use Change to set a working one");
    assert.equal(
      humanError(403, "needs capability thread:write", "bearer"),
      "Your token is not allowed to do this; it needs thread:write. Mint a token with it in Tokens",
    );
    // Not tied to the credential: the same words for everyone.
    for (const mode of ["session", "bearer", "delegated"]) {
      assert.equal(humanError(404, "", mode), humanError(404, "", null));
      assert.equal(humanError(500, "", mode), humanError(500, "", null));
    }
  });

  test("humanError follows the mode the page set, and forgets it on reset", () => {
    setIdentityMode("session");
    try {
      assert.match(humanError(403, "forbidden"), /^Your session is not allowed/);
      setIdentityMode("delegated");
      assert.match(humanError(403, "forbidden"), /^This delegated token is not allowed/);
      setIdentityMode("something else");
      assert.equal(humanError(401, ""), "Your token or session was not accepted. Use Change to set a working one");
    } finally {
      setIdentityMode(null);
    }
  });
});


describe("toasts, discovery, and the image cache", { concurrency: 1 }, () => {
  test("the same error again is cleared and put back, so a screen reader says it again", (t) => {
    // Cleared before the mock, so a real timer left by an earlier test is
    // really cancelled.
    clearToasts();
    t.mock.timers.enable({ apis: ["setTimeout"] });
    showError("Channel name required");
    const region = document.getElementById("toasts");
    const text = region.children[0].children[0];
    assert.equal(text.textContent, "Channel name required");
    showError("Channel name required");
    assert.equal(region.children.length, 1, "one toast, not a stack");
    assert.equal(text.textContent, "", "the text is cleared first");
    t.mock.timers.tick(100);
    assert.equal(text.textContent, "Channel name required", "then put back in a later task");
    showError("Channel name required");
    showError("Channel name required");
    t.mock.timers.tick(100);
    assert.equal(text.textContent, "Channel name required", "a pending restore is replaced, not stacked");
    clearToasts();
  });

  test("showError takes a severity: an error alerts, a success is a polite status", (t) => {
    clearToasts();
    t.mock.timers.enable({ apis: ["setTimeout"] });
    // The stub drops attributes and never detaches, so record the roles the
    // toasts are given and let remove() take a toast out of the region.
    const region = document.getElementById("toasts");
    const roles = new Map();
    const make = document.createElement;
    t.mock.method(document, "createElement", (tag) => {
      const el = make.call(document, tag);
      el.setAttribute = (name, value) => {
        if (name === "role") roles.set(el, value);
      };
      el.remove = () => {
        const at = region.children.indexOf(el);
        if (at >= 0) region.children.splice(at, 1);
      };
      return el;
    });
    showError("Could not post: The server hit an error");
    showError("Token minted", "success");
    showError("Attenuation: request exceeds your grant", "warning");
    showError("Token minted");
    showError("Odd severity", "shout");
    const shown = region.children.map((toast) => [toast.dataset.message, toast.dataset.severity, toast.className, roles.get(toast)]);
    assert.deepEqual(shown, [
      ["Attenuation: request exceeds your grant", "warning", "toast toast-warning", "alert"],
      ["Token minted", "error", "toast toast-error", "alert"],
      ["Odd severity", "error", "toast toast-error", "alert"],
    ], "three at most, oldest dropped first; an unknown severity is an error");
    clearToasts();
    showError("Token minted", "success");
    assert.equal(roles.get(region.children[0]), "status", "a success does not interrupt");
    assert.equal(region.children[0].className, "toast toast-success");
    showError("Token minted", "success");
    assert.equal(region.children.length, 1, "the same success again is one toast");
    showError("Token minted");
    assert.equal(region.children.length, 2, "an error with the same words is its own toast");
    clearToasts();
  });

  test("an older discovery answer never overrides the API base set after it", async () => {
    let releaseOld;
    globalThis.fetch = async (url) => {
      if (String(url).startsWith("http://old.test")) {
        return new Promise((resolve) => {
          releaseOld = () =>
            resolve(jsonResponse({ auth: { oidc: true, oidc_login: "/auth/oidc/login", sessions: true } }));
        });
      }
      return jsonResponse({ auth: { oidc: false, sessions: false } });
    };
    document.getElementById("base").value = "http://old.test";
    const older = loadServerAuth();
    document.getElementById("base").value = BASE;
    await loadServerAuth();
    releaseOld();
    await older;
    assert.equal(session.oidcLoginPath, null);
    assert.equal(session.serverOffersSessions, false);
    assert.equal(document.getElementById("first-run-oidc").hidden, true);
  });

  test("a discovery read that got no answer is tried again; a 404 is an answer", async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    document.getElementById("base").value = BASE;
    let calls = 0;
    globalThis.fetch = async () => {
      calls += 1;
      if (calls === 1) throw new TypeError("Failed to fetch");
      return jsonResponse({ auth: { oidc: true, oidc_login: "/auth/oidc/login", sessions: true } });
    };
    await loadServerAuth();
    assert.equal(session.oidcLoginPath, null);
    assert.equal(document.getElementById("first-run-oidc").hidden, true);
    t.mock.timers.tick(2000);
    await settle();
    assert.equal(calls, 2);
    assert.equal(session.oidcLoginPath, "/auth/oidc/login");
    assert.equal(document.getElementById("first-run-oidc").hidden, false);

    calls = 0;
    globalThis.fetch = async () => {
      calls += 1;
      return jsonResponse({}, 404);
    };
    await loadServerAuth();
    t.mock.timers.tick(60000);
    await settle();
    assert.equal(calls, 1, "no discovery document is not retried");
    assert.equal(session.oidcLoginPath, null);
  });

  test("a stalled discovery read does not hold up signing in", async () => {
    document.getElementById("base").value = BASE;
    document.getElementById("token").value = "";
    const seen = [];
    globalThis.fetch = async (url) => {
      const u = String(url);
      seen.push(u);
      if (u.endsWith("/.well-known/maidan.json")) return new Promise(() => {});
      return jsonResponse({}, 401);
    };
    const started = session.start().then(() => "started");
    let timer;
    const stalled = new Promise((resolve) => (timer = setTimeout(() => resolve("stalled"), 1000)));
    const first = await Promise.race([started, stalled]);
    clearTimeout(timer);
    assert.equal(first, "started");
    assert.ok(seen.some((u) => u.endsWith("/auth/session")), "the session was checked");
  });

  test("the image cache evicts the least recently used past its byte budget", () => {
    const images = new Map([["a", 1], ["b", 2], ["c", 3]]);
    const sizes = new Map([["a", 40], ["b", 40], ["c", 40]]);
    trimImageCache(images, sizes, 100, 10, "c");
    assert.deepEqual([...images.keys()], ["b", "c"]);
    assert.deepEqual([...sizes.keys()], ["b", "c"]);

    // The image being drawn stays even when it alone is over the budget.
    images.set("huge", 4);
    sizes.set("huge", 500);
    trimImageCache(images, sizes, 100, 10, "huge");
    assert.deepEqual([...images.keys()], ["huge"]);

    // The entry limit still holds when nothing has a size yet.
    const pending = new Map([["x", 1], ["y", 2], ["z", 3]]);
    trimImageCache(pending, new Map(), 100, 2, "z");
    assert.deepEqual([...pending.keys()], ["y", "z"]);
  });
});


describe("browser web push registration", { concurrency: 1 }, () => {
  test("the service worker listens for a push", () => {
    const sw = readFileSync(new URL("./sw.js", import.meta.url), "utf8");
    assert.match(sw, /addEventListener\("push"/);
    assert.match(sw, /showNotification/);
  });

  test("an unset VAPID key does not register a subscription", async () => {
    await resetAuth();
    const calls = [];
    globalThis.fetch = async (url, opts) => {
      calls.push({ url: String(url), method: opts && opts.method });
      return {
        ok: true,
        status: 200,
        json: async () => ({ reason: "vapid_unset" }),
        text: async () => "",
      };
    };
    const result = await registerBrowserPush("mem_1", {
      notification: { permission: "granted", requestPermission: async () => "granted" },
      serviceWorker: { register: async () => { throw new Error("should not register"); } },
    });
    assert.deepEqual(result, { registered: false, reason: "vapid_unset" });
    assert.equal(calls.length, 1);
    assert.match(calls[0].url, /\/web-push\/vapid-public-key$/);
  });

  test("the board registers the service worker and stores the subscription", async () => {
    await resetAuth();
    document.getElementById("token").value = "sekrit";
    const posts = [];
    let registeredPath = "";
    globalThis.fetch = async (url, opts) => {
      const u = String(url);
      if (u.endsWith("/web-push/vapid-public-key")) {
        return {
          ok: true,
          status: 200,
          json: async () => ({ public_key: "AQ" }),
          text: async () => "",
        };
      }
      posts.push({ url: u, body: JSON.parse(opts.body) });
      return {
        ok: true,
        status: 200,
        json: async () => ({ id: "sub_1" }),
        text: async () => "",
      };
    };
    const subscription = {
      endpoint: "https://push.example/device",
      toJSON() {
        return { endpoint: this.endpoint, keys: { p256dh: "k", auth: "a" } };
      },
    };
    const result = await registerBrowserPush("mem_1", {
      notification: { permission: "granted", requestPermission: async () => "granted" },
      serviceWorker: {
        async register(path) {
          registeredPath = path;
          return {
            pushManager: {
              getSubscription: async () => null,
              subscribe: async () => subscription,
            },
          };
        },
      },
    });
    assert.deepEqual(result, { registered: true, reason: "registered" });
    assert.equal(registeredPath, "/ui/static/sw.js");
    assert.equal(posts.length, 1);
    assert.match(posts[0].url, /\/members\/mem_1\/push-subscriptions$/);
    assert.equal(posts[0].body.endpoint, "https://push.example/device");
    assert.deepEqual(posts[0].body.keys, { p256dh: "k", auth: "a" });
  });
});
