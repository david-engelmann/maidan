import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  SITEVERIFY_URL,
  TRY_CONFIG_PATH,
  TRY_PATHS,
  TURNSTILE_SECRET_VAR,
  TURNSTILE_SITE_KEY_VAR,
  checkTurnstile,
  handleTry,
  tryCsp,
} from "../src/try-page.js";
import { claudeCommand, codexConfig, cursorLink, vscodeLink } from "../try/clients.js";

const page = readFileSync(new URL("../try/index.html", import.meta.url), "utf8");
const script = readFileSync(new URL("../try/try.js", import.meta.url), "utf8");

const URL_ = "https://try.maidan.world/mcp/streamable";
const TOKEN = "mdn_secret_abc";

// A stand-in for the Worker's static-assets binding.
function assets() {
  const asked = [];
  return {
    asked,
    ASSETS: {
      async fetch(req) {
        asked.push(new URL(req.url).pathname);
        return new Response("asset", { headers: { "content-type": "text/plain" } });
      },
    },
  };
}

test("every file the page loads is served by the Worker, not the container", () => {
  for (const ref of [...page.matchAll(/(?:href|src)="(\/[^"]*)"/g)].map((m) => m[1])) {
    assert.ok(TRY_PATHS.has(ref), `${ref} is not in TRY_PATHS`);
  }
  for (const ref of [...script.matchAll(/from "(\/[^"]+)"/g)].map((m) => m[1])) {
    assert.ok(TRY_PATHS.has(ref), `${ref} is not in TRY_PATHS`);
  }
  const css = readFileSync(new URL("../try/try.css", import.meta.url), "utf8");
  for (const ref of [...css.matchAll(/url\("(\/[^"]+)"\)/g)].map((m) => m[1])) {
    assert.ok(TRY_PATHS.has(ref), `${ref} is not in TRY_PATHS`);
  }
});

test("the page is answered from assets with a CSP that runs only its own script", async () => {
  const env = assets();
  const res = await handleTry(new Request("https://try.maidan.world/"), env);
  assert.ok(res);
  assert.deepEqual(env.asked, ["/"]);
  const csp = res.headers.get("content-security-policy");
  assert.match(csp, /script-src 'self'(;|$)/);
  assert.match(csp, /connect-src 'self'/);
  assert.match(csp, /frame-ancestors 'none'/);
  assert.doesNotMatch(csp, /unsafe-inline|unsafe-eval/);
  assert.equal(res.headers.get("cache-control"), "no-store");
});

test("the page carries no inline script or style, which its CSP would block", () => {
  assert.doesNotMatch(page, /<script(?![^>]*\bsrc=)[^>]*>/);
  assert.doesNotMatch(page, /\sstyle=|<style/);
  assert.doesNotMatch(page, /\son[a-z]+=/);
});

test("the page says the sandbox is deleted within 24 hours and points at the waitlist", () => {
  assert.match(page, /This sandbox and everything in it are deleted within 24 hours\./);
  assert.match(page, /Want to keep your work\? <a href="https:\/\/maidan\.world\/#hosting">Join the waitlist at maidan\.world\.<\/a>/);
  assert.match(page, /Start a sandbox/);
});

test("the admin token never goes into a URL: the console sign-in is a POST with a header", () => {
  assert.match(script, /fetch\("\/auth\/session\/from-token", \{\s*method: "POST",\s*headers: \{[^}]*authorization: `Bearer \$\{adminToken\}`/);
  assert.doesNotMatch(script, /location[^;\n]*adminToken|\?[^"'`\n]*adminToken|#[^"'`\n]*adminToken/);
  assert.doesNotMatch(script, /localStorage|sessionStorage|document\.cookie/);
});

test("other paths go to the container", async () => {
  const env = assets();
  for (const [method, path] of [["GET", "/health"], ["POST", "/mcp/streamable"], ["GET", "/ui/"]]) {
    assert.equal(await handleTry(new Request(`https://try.maidan.world${path}`, { method }), env), null);
  }
  assert.deepEqual(env.asked, []);
});

test("without Turnstile configured, the page shows no widget and the mint takes no token", async () => {
  const env = assets();
  const res = await handleTry(new Request(`https://try.maidan.world${TRY_CONFIG_PATH}`), env);
  assert.deepEqual(await res.json(), { turnstile_site_key: null });
  const mint = new Request("https://try.maidan.world/sandbox", { method: "POST", body: "{}" });
  assert.equal(await handleTry(mint, env, () => assert.fail("no siteverify call")), null);
  assert.doesNotMatch(tryCsp(false), /challenges\.cloudflare\.com/);
});

function turnstileEnv() {
  return { ...assets(), [TURNSTILE_SITE_KEY_VAR]: "site-key", [TURNSTILE_SECRET_VAR]: "secret" };
}

function mintWith(token) {
  return new Request("https://try.maidan.world/sandbox", {
    method: "POST",
    headers: { "content-type": "application/json", "cf-connecting-ip": "203.0.113.9" },
    body: JSON.stringify(token === undefined ? {} : { turnstile_token: token }),
  });
}

function siteverify(answer, seen = []) {
  return async (url, init) => {
    seen.push({ url, form: Object.fromEntries(init.body.entries()) });
    return new Response(JSON.stringify(answer), { headers: { "content-type": "application/json" } });
  };
}

test("with Turnstile, a mint passes only on a token Cloudflare accepts for this host and action", async () => {
  const env = turnstileEnv();
  const seen = [];
  const ok = siteverify({ success: true, hostname: "try.maidan.world", action: "sandbox" }, seen);
  const req = mintWith("tok");
  assert.equal(await checkTurnstile(req, env, ok), null);
  assert.equal(seen[0].url, SITEVERIFY_URL);
  assert.deepEqual(seen[0].form, { secret: "secret", response: "tok", remoteip: "203.0.113.9" });
  // The body is still there for the container.
  assert.deepEqual(await req.json(), { turnstile_token: "tok" });

  for (const answer of [
    { success: false, hostname: "try.maidan.world", action: "sandbox" },
    { success: true, hostname: "elsewhere.example", action: "sandbox" },
    { success: true, hostname: "try.maidan.world", action: "waitlist" },
  ]) {
    const res = await checkTurnstile(mintWith("tok"), env, siteverify(answer));
    assert.equal(res.status, 403, JSON.stringify(answer));
  }
  assert.equal((await checkTurnstile(mintWith(undefined), env, ok)).status, 403);
  assert.equal((await checkTurnstile(mintWith(""), env, ok)).status, 403);
  const down = async () => {
    throw new Error("unreachable");
  };
  assert.equal((await checkTurnstile(mintWith("tok"), env, down)).status, 502);
});

test("with Turnstile, the page's CSP lets the widget load and the config names the site key", async () => {
  const env = turnstileEnv();
  const csp = (await handleTry(new Request("https://try.maidan.world/"), env)).headers.get("content-security-policy");
  assert.match(csp, /script-src 'self' https:\/\/challenges\.cloudflare\.com/);
  assert.match(csp, /frame-src https:\/\/challenges\.cloudflare\.com/);
  const config = await (await handleTry(new Request(`https://try.maidan.world${TRY_CONFIG_PATH}`), env)).json();
  assert.deepEqual(config, { turnstile_site_key: "site-key" });
  assert.doesNotMatch(JSON.stringify(config), /secret/);
});

test("the Claude Code command adds the HTTP server with the token as a header", () => {
  assert.equal(
    claudeCommand(URL_, TOKEN),
    `claude mcp add --transport http maidan ${URL_} --header "Authorization: Bearer ${TOKEN}"`,
  );
});

test("the Cursor link is Cursor's install deeplink with a base64 mcp.json entry", () => {
  const link = new URL(cursorLink(URL_, TOKEN));
  assert.equal(link.protocol, "cursor:");
  assert.equal(`${link.host}${link.pathname}`, "anysphere.cursor-deeplink/mcp/install");
  assert.equal(link.searchParams.get("name"), "maidan");
  const config = JSON.parse(Buffer.from(link.searchParams.get("config"), "base64").toString("utf8"));
  assert.deepEqual(config, { url: URL_, headers: { Authorization: `Bearer ${TOKEN}` } });
});

test("the VS Code link is vscode:mcp/install with the JSON code --add-mcp takes", () => {
  const link = vscodeLink(URL_, TOKEN);
  assert.ok(link.startsWith("vscode:mcp/install?"));
  const config = JSON.parse(decodeURIComponent(link.slice("vscode:mcp/install?".length)));
  assert.deepEqual(config, { name: "maidan", type: "http", url: URL_, headers: { Authorization: `Bearer ${TOKEN}` } });
});

test("the Codex config names an environment variable rather than holding the token", () => {
  const text = codexConfig(URL_, TOKEN);
  assert.match(text, /^\[mcp_servers\.maidan\]$/m);
  assert.match(text, new RegExp(`^url = "${URL_.replace(/[.\/]/g, "\\$&")}"$`, "m"));
  assert.match(text, /^bearer_token_env_var = "MAIDAN_TOKEN"$/m);
  assert.match(text, new RegExp(`^export MAIDAN_TOKEN=${TOKEN}$`, "m"));
});
