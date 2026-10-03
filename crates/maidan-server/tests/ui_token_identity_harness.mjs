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
globalThis.fetch = async (url) => {
  const target = String(url);
  if (target.includes("/auth/session/from-token")) {
    return { ok: false, status: 404, json: async () => ({}), text: async () => "" };
  }
  if (target.endsWith("/auth/session")) {
    return { ok: false, status: 401, json: async () => ({}), text: async () => "" };
  }
  if (target.endsWith("/me")) {
    if (meThrows) throw new TypeError("network down");
    return {
      ok: true,
      status: 200,
      json: async () => ({ member_id: "mem_old", workspace_id: "ws_1" }),
      text: async () => "",
    };
  }
  throw new Error("unexpected fetch " + target);
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
