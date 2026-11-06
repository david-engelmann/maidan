// The official MCP TypeScript SDK as a client of a seeded Maidan.
//
// Usage: node client.mjs <base-url> <endpoint> <bearer-token>
// Connects over Streamable HTTP, lists the tools, then walks the room the way
// an agent would: whoami, create a channel and a thread, post a message, and
// read it back. Exits non-zero on the first check that fails.
//
// Argument checks run before the SDK is imported so a bad URL/token fails
// closed without needing `npm ci`.

const ALLOWED_ENDPOINTS = new Set(["/mcp", "/mcp/streamable"]);

function fail(msg) {
  console.error(`FAIL (typescript): ${msg}`);
  process.exit(1);
}

/** Refuse non-loopback cleartext and non-http(s) bases; allowlist MCP paths. */
export function assertClientArgs(base, endpoint, token) {
  if (!base || !endpoint || !token) {
    fail("usage: client.mjs <base-url> <endpoint> <bearer-token>");
  }
  let url;
  try {
    url = new URL(base);
  } catch {
    fail(`base-url is not a URL: ${base}`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    fail(`base-url must be http(s): ${base}`);
  }
  if (url.protocol === "http:") {
    const host = url.hostname.toLowerCase();
    if (host !== "127.0.0.1" && host !== "localhost" && host !== "::1") {
      fail(`refusing cleartext base-url off loopback: ${base}`);
    }
  }
  if (url.username || url.password) {
    fail(`base-url must not embed credentials: ${base}`);
  }
  if (!ALLOWED_ENDPOINTS.has(endpoint)) {
    fail(`endpoint must be /mcp or /mcp/streamable (got ${endpoint})`);
  }
  if (typeof token !== "string" || token.trim().length < 8 || /\s/.test(token)) {
    fail("bearer-token looks empty or malformed");
  }
}

const [base, endpoint, token] = process.argv.slice(2);
assertClientArgs(base, endpoint, token);

const { Client } = await import("@modelcontextprotocol/sdk/client/index.js");
const { StreamableHTTPClientTransport } = await import(
  "@modelcontextprotocol/sdk/client/streamableHttp.js"
);

const transport = new StreamableHTTPClientTransport(new URL(base + endpoint), {
  requestInit: { headers: { Authorization: `Bearer ${token}` } },
});
const client = new Client({ name: "maidan-sdk-client-check", version: "0" });

async function call(name, args) {
  const result = await client.callTool({ name, arguments: args });
  if (result.isError) fail(`${name} ${JSON.stringify(args)}: ${JSON.stringify(result)}`);
  return JSON.parse(result.content[0].text);
}

try {
  await client.connect(transport);
  console.log(`  ok  initialize: ${client.getServerVersion()?.name}`);
  const { tools } = await client.listTools();
  if (tools.length < 100) fail(`tools/list returned ${tools.length} tools`);
  console.log(`  ok  tools/list: ${tools.length} tools`);
  const me = await call("whoami", {});
  const channel = (
    await call("create_channel", {
      workspace_id: me.workspace_id,
      name: `sdk-typescript${endpoint.replaceAll("/", "-")}`,
    })
  ).id;
  const thread = (await call("create_thread", { channel_id: channel, title: "from the typescript sdk" })).id;
  await call("post_message", { thread_id: thread, body: "hello from typescript" });
  const listed = JSON.stringify(await call("list_messages", { thread_id: thread }));
  if (!listed.includes("hello from typescript")) fail(`list_messages did not return the posted message: ${listed}`);
  console.log("  ok  whoami, create_channel, create_thread, post_message, list_messages");
  // A bad argument may come back as a tool error or a JSON-RPC invalid-params
  // error; either way it must name the argument.
  let refusal;
  try {
    const bad = await client.callTool({ name: "get_thread_context", arguments: { thread_id: "not-a-uuid" } });
    if (!bad.isError) fail("an invalid thread_id was accepted");
    refusal = JSON.stringify(bad.content);
  } catch (err) {
    refusal = String(err?.message ?? err);
  }
  if (!refusal.includes("thread_id")) fail(`the refusal does not name thread_id: ${refusal}`);
  console.log("  ok  an invalid argument is refused, naming it");
  await client.close();
} catch (err) {
  fail(err?.stack ?? String(err));
}
