// Drives the pasted-token handler in session.js. Node has no document, so
// the stub is installed before the module loads. The handler is the one
// the page registers. This file is not a copy of that handler.
import assert from "node:assert/strict";

function makeEl() {
  const children = [];
  const listeners = {};
  return {
    value: "",
    hidden: false,
    className: "",
    textContent: "",
    title: "",
    tabIndex: 0,
    open: false,
    method: "",
    action: "",
    dataset: {},
    style: {},
    children,
    listeners,
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
    addEventListener(type, fn) {
      (listeners[type] = listeners[type] || []).push(fn);
    },
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

const ui = new URL("../static/ui/session.js", import.meta.url).href;
const people = new URL("../static/ui/people.js", import.meta.url).href;
const session = await import(ui);
const { refreshBearerIdentity } = session;
const { authorId } = await import(people);

let meThrows = false;
// A /me answer held back until the test releases it, keyed by the token.
const heldMe = new Map();
globalThis.fetch = async (url, options) => {
  const target = String(url);
  if (target.includes("/auth/session/from-token")) {
    return { ok: false, status: 404, json: async () => ({}), text: async () => "" };
  }
  if (target.endsWith("/auth/session")) {
    return { ok: false, status: 401, json: async () => ({}), text: async () => "" };
  }
  if (target.endsWith("/me")) {
    const auth = (options && options.headers && options.headers.Authorization) || "";
    const held = heldMe.get(auth.replace(/^Bearer /, ""));
    if (held) return held;
    if (meThrows) throw new TypeError("network down");
    const member = auth === "Bearer secret-b" ? "mem_b" : "mem_old";
    return {
      ok: true,
      status: 200,
      json: async () => ({ member_id: member, workspace_id: "ws_1" }),
      text: async () => "",
    };
  }
  // Signing in goes on to load the board; those reads answer empty.
  return { ok: true, status: 200, json: async () => [], text: async () => "[]" };
};

document.getElementById("workspace").value = "ws_1";
document.getElementById("token").value = "secret-old";
const identity = await refreshBearerIdentity();
assert.equal(identity.member_id, "mem_old");
assert.equal(session.bearerMemberId, "mem_old");
assert.equal(authorId(), "mem_old");

meThrows = true;
document.getElementById("token").value = "secret-new";
const change = document.getElementById("token").listeners.change;
assert.ok(change && change.length >= 1, "the page registers a token change handler");
await Promise.all(change.map((fn) => fn()));

assert.equal(session.bearerMemberId, null);
assert.equal(authorId(), null);
const status = document.getElementById("session-status");
assert.equal(status.hidden, false);
assert.equal(status.className, "err");
assert.match(status.textContent, /^Could not reach the server at /);

// Token A is pasted and its check is held. While it runs, the field and the
// Sign in button are locked, so token B cannot race it (#1257 serializes
// sign-in). A second change while A is in flight joins A's attempt rather
// than starting another. A's rejection is shown, then B signs in cleanly
// and clears it.
meThrows = false;
let releaseA;
heldMe.set(
  "secret-a",
  new Promise((resolve) => {
    releaseA = () => resolve({ ok: false, status: 401, json: async () => ({}), text: async () => "" });
  }),
);
document.getElementById("token").value = "secret-a";
const pastedA = Promise.all(change.map((fn) => fn()));
await new Promise((resolve) => setTimeout(resolve, 0));
assert.equal(document.getElementById("token").disabled, true, "the token field is locked while a sign-in runs");
assert.equal(document.getElementById("token-signin").disabled, true, "Sign in is locked while a sign-in runs");
releaseA();
await pastedA;
assert.equal(document.getElementById("token").disabled, false, "the field unlocks when the attempt ends");
assert.equal(status.className, "err", "token A's rejection is shown");
assert.equal(session.bearerMemberId, null);
document.getElementById("token").value = "secret-b";
await Promise.all(change.map((fn) => fn()));
assert.equal(session.bearerMemberId, "mem_b");
assert.equal(authorId(), "mem_b");
assert.notEqual(status.className, "err", "token B's success clears token A's rejection");

// The page keeps retrying its live socket by design, and Node has no socket
// server here, so end once every assertion has passed.
process.exit(0);
