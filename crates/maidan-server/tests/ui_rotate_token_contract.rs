//! A token rotation that is still in flight must not install its secret into
//! a connection that has since changed. Open Work item 3 (review on #1127):
//! activate the returned secret only when the token, the API base, the
//! workspace, and the token id are still the ones the request started with.
//! The secret is still shown once. A network failure is a sentence, not an
//! unhandled rejection. Changing the token field clears the cached token id.

use serde_json::json;

const TOOLS: &str = include_str!("../static/ui/tools.js");

#[test]
fn rotate_token_activates_only_when_the_connection_is_unchanged() {
    let report = run_cases();
    let cases = report.as_array().expect("case list");
    assert!(!cases.is_empty(), "the page ran no rotation cases");
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let want_exchange = case["want_exchange"].as_bool().expect("want");
        let exchanged = !case["exchanged"].is_null();
        assert_eq!(
            exchanged, want_exchange,
            "{name}: exchange was {exchanged}, secret {:?}",
            case["exchanged"]
        );
        assert_eq!(
            case["shown"], case["want_shown"],
            "{name}: the one-time secret"
        );
        assert_eq!(
            case["token_id"], case["want_token_id"],
            "{name}: cached token id"
        );
        assert_eq!(
            case["token_field"], case["want_token_field"],
            "{name}: token field"
        );
        if case["want_error"].is_null() {
            assert!(
                case["error"].is_null(),
                "{name}: unexpected error {:?}",
                case["error"]
            );
        } else {
            let error = case["error"].as_str().unwrap_or("");
            let needle = case["want_error"].as_str().expect("needle");
            assert!(
                error.contains(needle),
                "{name}: error {error:?} does not contain {needle:?}"
            );
        }
        if name == "same-connection" {
            assert_eq!(case["method"], "POST", "{name}: method");
            let url = case["url"].as_str().unwrap_or("");
            assert!(url.contains("/tokens/tok-1/rotate"), "{name}: url {url}");
            assert_eq!(case["reconnected"], 1, "{name}: the live socket reconnects");
        }
        if name == "uppercase-id" {
            assert_eq!(case["reconnected"], 1, "{name}: the live socket reconnects");
        }
        if !["same-connection", "exchange-failed", "uppercase-id"].contains(&name) {
            assert_eq!(case["reconnected"], 0, "{name}: socket left alone");
        }
    }
    let cleared = report
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "token-field-change")
        .expect("token field change");
    assert_eq!(cleared["token_id"], json!(null));
}

fn run_cases() -> serde_json::Value {
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to run rotateToken: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &json!({ "src": TOOLS }))
        .expect("write tools.js");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "rotateToken harness failed\n{stderr}\n{stdout}"
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("rotateToken harness returned {err}: {stdout}"))
}

const HARNESS: &str = r##"
const fs = require("fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const src = input.src;
function grab(name) {
  const key = "async function " + name + "(";
  const at = src.indexOf(key);
  if (at < 0) throw new Error("missing " + name);
  const rel = src.slice(at).indexOf("\n      }\n");
  if (rel < 0) throw new Error("unclosed " + name);
  return src.slice(at, at + rel) + "\n      }";
}
const listenerAt = src.indexOf("const tokenField = document.getElementById(\"token\")");
if (listenerAt < 0) throw new Error("missing token field listener");
const listenerRel = src.slice(listenerAt).indexOf("\n      }\n");
if (listenerRel < 0) throw new Error("unclosed token field listener");
const listener = src.slice(listenerAt, listenerAt + listenerRel) + "\n      }";
const rotateSource = grab("rotateToken");

const state = {
  token: "sekret",
  base: "http://localhost:8080",
  workspace: "ws-1",
  currentTokenId: "tok-1",
};
const obs = {
  exchanged: null,
  shown: null,
  error: null,
  url: null,
  method: null,
  reconnected: 0,
  tokenField: "sekret",
};
let wsSocket = { ready: true };
function token() { return state.token; }
function base() { return state.base; }
function wid() { return state.workspace; }
function headers() { return { Authorization: "Bearer " + state.token }; }
function persist() {}
function showSecretOnce(_title, secret) { obs.shown = secret; }
function showError(msg) { obs.error = msg; }
function setStatus() {}
function setOut() {}
function disconnectWs() {}
function connectWs() { obs.reconnected += 1; }
function unreachable() { return "Could not reach the server"; }
async function responseError() { return "Could not rotate that token"; }
async function exchangeToken(secret) {
  obs.exchanged = secret;
  return state.exchangeResult || { ok: true };
}
const tokenEl = { value: "sekret" };
const document = {
  getElementById(id) {
    if (id === "token") return tokenEl;
    return { value: "" };
  },
};
let apiImpl = async () => ({
  ok: true,
  async json() { return { id: "tok-2", secret: "new-secret" }; },
});
async function api(url, options) {
  obs.url = url;
  obs.method = options && options.method;
  return apiImpl(url, options);
}
async function writeApi(_button, url, options) {
  return api(url, options);
}

const runRotate = new Function(
  "deps",
  "let currentTokenId = deps.currentTokenId;\n" +
  "let wsSocket = deps.wsSocket;\n" +
  "const token = deps.token;\n" +
  "const base = deps.base;\n" +
  "const wid = deps.wid;\n" +
  "const headers = deps.headers;\n" +
  "const persist = deps.persist;\n" +
  "const showSecretOnce = deps.showSecretOnce;\n" +
  "const showError = deps.showError;\n" +
  "const setStatus = deps.setStatus;\n" +
  "const setOut = deps.setOut;\n" +
  "const disconnectWs = deps.disconnectWs;\n" +
  "const connectWs = deps.connectWs;\n" +
  "const unreachable = deps.unreachable;\n" +
  "const responseError = deps.responseError;\n" +
  "const exchangeToken = deps.exchangeToken;\n" +
  "const document = deps.document;\n" +
  "const api = deps.api;\n" +
  "const writeApi = deps.writeApi;\n" +
  rotateSource + "\n" +
  "return async function (id) {\n" +
  "  await rotateToken(id);\n" +
  "  return currentTokenId;\n" +
  "};"
);

const deps = {
  token, base, wid, headers, persist, showSecretOnce, showError, setStatus, setOut,
  disconnectWs, connectWs, unreachable, responseError, exchangeToken, document, api, writeApi,
  currentTokenId: state.currentTokenId,
  wsSocket,
};

function reset() {
  state.token = "sekret";
  state.base = "http://localhost:8080";
  state.workspace = "ws-1";
  state.exchangeResult = { ok: true };
  tokenEl.value = "sekret";
  obs.exchanged = null;
  obs.shown = null;
  obs.error = null;
  obs.url = null;
  obs.method = null;
  obs.reconnected = 0;
  deps.currentTokenId = "tok-1";
  deps.wsSocket = { ready: true };
  apiImpl = async () => ({
    ok: true,
    async json() { return { id: "tok-2", secret: "new-secret" }; },
  });
}

const cases = [];
async function scenario(name, setup, want, id = "tok-1") {
  reset();
  setup();
  const rotate = runRotate(deps);
  const tokenId = await rotate(id);
  cases.push({
    name,
    exchanged: obs.exchanged,
    shown: obs.shown,
    error: obs.error,
    url: obs.url,
    method: obs.method,
    reconnected: obs.reconnected,
    token_id: tokenId,
    token_field: tokenEl.value,
    want_exchange: want.exchange,
    want_shown: want.shown,
    want_token_id: want.tokenId,
    want_token_field: want.tokenField,
    want_error: want.error,
  });
}

(async () => {
  await scenario("same-connection", () => {}, {
    exchange: true,
    shown: "new-secret",
    tokenId: "tok-2",
    tokenField: "sekret",
    error: null,
  });
  // Tokens takes the id as typed; the server reads any case.
  await scenario("uppercase-id", () => {
    deps.currentTokenId = "0192f3a4-5b6c-7d8e-9f00-aabbccddeeff";
  }, {
    exchange: true,
    shown: "new-secret",
    tokenId: "tok-2",
    tokenField: "sekret",
    error: null,
  }, "0192F3A4-5B6C-7D8E-9F00-AABBCCDDEEFF");
  await scenario("workspace-changed", () => {
    apiImpl = async () => {
      state.workspace = "ws-2";
      return { ok: true, async json() { return { id: "tok-9", secret: "other-secret" }; } };
    };
  }, {
    exchange: false,
    shown: "other-secret",
    tokenId: "tok-1",
    tokenField: "sekret",
    error: null,
  });
  await scenario("token-changed", () => {
    apiImpl = async () => {
      state.token = "other-token";
      return { ok: true, async json() { return { id: "tok-9", secret: "other-secret" }; } };
    };
  }, {
    exchange: false,
    shown: "other-secret",
    tokenId: "tok-1",
    tokenField: "sekret",
    error: null,
  });
  await scenario("base-changed", () => {
    apiImpl = async () => {
      state.base = "http://other.example";
      return { ok: true, async json() { return { id: "tok-9", secret: "other-secret" }; } };
    };
  }, {
    exchange: false,
    shown: "other-secret",
    tokenId: "tok-1",
    tokenField: "sekret",
    error: null,
  });
  await scenario("other-token", () => {
    deps.currentTokenId = "tok-other";
  }, {
    exchange: false,
    shown: "new-secret",
    tokenId: "tok-other",
    tokenField: "sekret",
    error: null,
  });
  await scenario("network", () => {
    apiImpl = async () => { throw new Error("boom"); };
  }, {
    exchange: false,
    shown: null,
    tokenId: "tok-1",
    tokenField: "sekret",
    error: "Could not reach",
  });
  await scenario("http-error", () => {
    apiImpl = async () => ({ ok: false, status: 409, async json() { return {}; } });
  }, {
    exchange: false,
    shown: null,
    tokenId: "tok-1",
    tokenField: "sekret",
    error: "Could not rotate",
  });
  await scenario("exchange-failed", () => {
    state.exchangeResult = { ok: false, error: "Could not sign in with that token" };
  }, {
    exchange: true,
    shown: "new-secret",
    tokenId: "tok-2",
    tokenField: "new-secret",
    error: "Could not sign in",
  });

  // The token field listener clears the cached id. It is not inside rotateToken.
  const field = {
    addEventListener(type, fn) { this.fn = fn; this.type = type; },
  };
  const documentForField = {
    getElementById(id) { return id === "token" ? field : null; },
  };
  const install = new Function(
    "document",
    "let currentTokenId = \"tok-1\";\n" +
    listener + "\n" +
    "return function () {\n" +
    "  const el = document.getElementById(\"token\");\n" +
    "  el.fn();\n" +
    "  return currentTokenId;\n" +
    "};"
  );
  const fire = install(documentForField);
  const watched = fire();
  cases.push({
    name: "token-field-change",
    exchanged: null,
    shown: null,
    error: null,
    url: null,
    method: null,
    reconnected: 0,
    token_id: watched,
    token_field: null,
    want_exchange: false,
    want_shown: null,
    want_token_id: null,
    want_token_field: null,
    want_error: null,
  });

  process.stdout.write(JSON.stringify(cases));
})().catch((err) => {
  console.error(err && err.stack || err);
  process.exit(1);
});
"##;
