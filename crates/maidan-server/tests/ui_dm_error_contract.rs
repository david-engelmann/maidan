//! A person reading or sending a DM never sees a raw status or a thrown error.
//!
//! Opening a DM already says the refusal as a sentence. The list, the message
//! pane, and Send still painted `HTTP 403` or `TypeError`. This runs those
//! handlers from `static/ui/dm.js` with the page's `responseError`,
//! `unreachable` and `showError`, against a 403, a 500, and a fetch that
//! throws. A failed send is one error toast.

use serde_json::Value;

const DM_JS: &str = include_str!("../static/ui/dm.js");
const FEEDBACK_JS: &str = include_str!("../static/ui/feedback.js");

fn dm_errors() -> Value {
    let payload = serde_json::json!({ "dm": DM_JS, "feedback": FEEDBACK_JS });
    let mut child = std::process::Command::new("node")
        .arg("-e")
        .arg(HARNESS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("node is required to run the DM error contract: {err}"));
    serde_json::to_writer(child.stdin.take().expect("stdin"), &payload).expect("write sources");
    let out = child.wait_with_output().expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "DM error harness failed\n{stderr}\n{stdout}"
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("DM error harness returned {err}: {stdout}"))
}

fn raw_or_http(text: &str) -> Option<&'static str> {
    if text.contains("HTTP") {
        return Some("HTTP");
    }
    if text.contains('{') || text.contains('}') {
        return Some("JSON");
    }
    let lower = text.to_ascii_lowercase();
    for needle in [
        "typeerror",
        "failed to fetch",
        "see stack",
        "panic:",
        "sql error",
        "boom",
    ] {
        if lower.contains(needle) {
            return Some(needle);
        }
    }
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        let digit = bytes[i].is_ascii_digit();
        let start = i == 0 || !bytes[i - 1].is_ascii_digit();
        if digit && start && bytes[i + 1].is_ascii_digit() && bytes[i + 2].is_ascii_digit() {
            let end = i + 3;
            let end_ok = end == bytes.len() || !bytes[end].is_ascii_digit();
            if end_ok {
                let n = (bytes[i] - b'0') as u16 * 100
                    + (bytes[i + 1] - b'0') as u16 * 10
                    + (bytes[i + 2] - b'0') as u16;
                if (100..600).contains(&n) {
                    return Some("status code");
                }
            }
        }
        i += 1;
    }
    None
}

#[test]
fn dm_lists_panes_and_sends_hide_the_raw_error() {
    let report = dm_errors();
    let cases = report["cases"].as_array().expect("cases");
    assert_eq!(
        cases.len(),
        10,
        "every DM read and send failure was exercised"
    );
    let mut saw_capability = false;
    let mut saw_server = false;
    let mut saw_unreachable = false;
    let mut saw_refused = false;
    for case in cases {
        let name = case["name"].as_str().expect("name");
        let text = case["text"].as_str().expect("text");
        assert!(
            raw_or_http(text).is_none(),
            "{name} shows a raw error ({})",
            raw_or_http(text).unwrap_or("?")
        );
        assert!(!text.is_empty(), "{name} painted nothing");
        if text.contains("thread:transition") {
            saw_capability = true;
        }
        if text.contains("The server hit an error") {
            saw_server = true;
        }
        if text.to_lowercase().contains("reach the server") {
            saw_unreachable = true;
        }
        if text == "Refused" {
            saw_refused = true;
        }
        if name.starts_with("send-") {
            assert_eq!(
                case["severity"].as_str(),
                Some("error"),
                "{name} is one error toast"
            );
            assert_eq!(
                case["draft"].as_str(),
                Some("hello"),
                "{name} cleared the draft after a failure"
            );
        }
    }
    assert!(saw_capability, "a 403 names the missing capability");
    assert!(saw_server, "a 500 is a sentence, not the server body");
    assert!(saw_unreachable, "a fetch that throws is a sentence");
    assert!(saw_refused, "a 409 send is the sentence Refused");
}

const HARNESS: &str = r#####"
const fs = require("fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));

function functionSource(js, name) {
  const mark = "function " + name + "(";
  let i = js.indexOf(mark);
  if (i < 0) throw new Error("missing " + name);
  if (js.slice(i - 6, i) === "async ") i -= 6;
  const open = js.indexOf("{", i);
  let depth = 0;
  for (let j = open; j < js.length; j++) {
    const c = js[j];
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0) return js.slice(i, j + 1);
    }
  }
  throw new Error("unclosed " + name);
}

function makeEl(tag, id) {
  const el = {
    tag: tag,
    id: id || "",
    children: [],
    className: "",
    textContent: "",
    value: "",
    dataset: {},
  };
  el.setAttribute = () => {};
  el.append = (...nodes) => {
    el.children.push(...nodes);
  };
  el.remove = () => {
    const region = els.toasts;
    region.children = region.children.filter((child) => child !== el);
  };
  Object.defineProperty(el, "firstElementChild", { get: () => el.children[0] || null });
  el.appendChild = (child) => {
    el.children.push(child);
    return child;
  };
  el.replaceChildren = () => {
    el.children = [];
    el.textContent = "";
  };
  el.matches = (sel) => sel.split(",").map((part) => part.trim()).includes(el.tag);
  return el;
}

const els = {};
function keep(tag, id) {
  const el = makeEl(tag, id);
  els[id] = el;
  return el;
}
keep("ul", "dm-list");
keep("ul", "gdm-list");
keep("div", "dm-messages");
keep("div", "gdm-messages");
keep("div", "toasts");
const dmBody = keep("input", "dm-body");
const gdmBody = keep("input", "gdm-body");
dmBody.value = "hello";
gdmBody.value = "hello";

const document = {
  getElementById(id) {
    if (!els[id]) els[id] = makeEl("div", id);
    return els[id];
  },
  createElement(tag) {
    return makeEl(tag, "");
  },
};

function visible(el) {
  const parts = [];
  function walk(node) {
    if (!node) return;
    if (node.children && node.children.length) {
      for (const child of node.children) walk(child);
    } else if (node.textContent) parts.push(String(node.textContent));
  }
  walk(el);
  return parts.join(" ").replace(/\s+/g, " ").trim();
}

function base() {
  return "http://maidan.test";
}
function authorId() {
  return "member-1";
}
async function loadMembers() {}
function wid() {
  return "ws-1";
}
function token() {
  return "";
}
function uiReadPath(path) {
  return base() + path;
}
function apiWritePath(path) {
  return base() + path;
}
function headers() {
  return {};
}
function requireAuthForWrite() {
  return true;
}
var selectedDm = { id: "dm1", thread_id: "thread-1" };
var selectedGdm = { id: "gdm1", thread_id: "thread-2" };

let scene = "refused";
async function api() {
  if (scene === "throw") throw new TypeError("Failed to fetch");
  if (scene === "server") {
    return { ok: false, status: 500, async text() { return "panic: sql error boom"; } };
  }
  if (scene === "conflict") {
    return { ok: false, status: 409, async text() { return JSON.stringify({ detail: "thread is not in review" }); } };
  }
  return {
    ok: false,
    status: 403,
    async text() {
      return JSON.stringify({ detail: "caller needs thread:transition; see stack {\"error\":\"boom\"}" });
    },
  };
}
async function writeApi(_button, url, options) {
  return api(url, options);
}

const feedback = input.feedback
  .split("\n")
  .filter((line) => {
    const trimmed = line.trim();
    return !trimmed.startsWith("import ") && !trimmed.startsWith("export ");
  })
  .join("\n");
eval(feedback);
for (const name of ["loadDms", "loadGroupDms", "loadConversationMessages", "sendDmMessage", "sendGroupDmMessage"]) {
  eval(functionSource(input.dm, name));
}

function reset() {
  for (const id of ["dm-list", "gdm-list", "dm-messages", "gdm-messages", "toasts"]) {
    els[id].children = [];
    els[id].textContent = "";
    els[id].className = "";
  }
  dmBody.value = "hello";
  gdmBody.value = "hello";
}

const cases = [];
async function run(name, which, next) {
  scene = which;
  reset();
  await next();
  // A failed send is a toast: its words, not the Dismiss button.
  const target = name.startsWith("messages-")
    ? "dm-messages"
    : name.startsWith("gdm-")
      ? "gdm-list"
      : "dm-list";
  const text = name.startsWith("send-")
    ? els.toasts.children.map((toast) => visible(toast.children[0])).join(" ")
    : visible(els[target]);
  const row = { name: name, text: text };
  if (name.startsWith("send-")) row.severity = els.toasts.children.map((toast) => toast.dataset.severity).join(" ");
  if (name.startsWith("send-dm")) row.draft = dmBody.value;
  if (name.startsWith("send-gdm")) row.draft = gdmBody.value;
  cases.push(row);
}

(async () => {
  await run("dms-403", "refused", () => loadDms());
  await run("dms-throw", "throw", () => loadDms());
  await run("gdm-403", "refused", () => loadGroupDms());
  await run("gdm-throw", "throw", () => loadGroupDms());
  await run("messages-500", "server", () => loadConversationMessages("thread-1", "dm-messages"));
  await run("messages-throw", "throw", () => loadConversationMessages("thread-1", "dm-messages"));
  await run("send-dm-409", "conflict", () => sendDmMessage());
  await run("send-dm-throw", "throw", () => sendDmMessage());
  await run("send-gdm-403", "refused", () => sendGroupDmMessage());
  await run("send-gdm-throw", "throw", () => sendGroupDmMessage());
  process.stdout.write(JSON.stringify({ cases: cases }));
})().catch((err) => {
  console.error(err && err.stack ? err.stack : err);
  process.exit(1);
});
"#####;
