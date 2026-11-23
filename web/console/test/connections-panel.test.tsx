import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { notices } from "../src/core/notices.ts";
import { SessionContext } from "../src/hooks/useSession.ts";
import { SessionStore } from "../src/core/session.ts";
import { Connections } from "../src/tools/Connections.tsx";
import { Tools } from "../src/tools/Tools.tsx";
import { FetchRouter, renderSignedIn, withSession, type Route } from "./console-harness.tsx";

const LIST = /\/workspaces\/ws\/connections$/;
const ONE = /\/workspaces\/ws\/connections\/(github|slack|linear)$/;
const TOKEN = "ghp_fixtureToken0123456789abcdefABCDEF";
const SECRET = "whsec-fixture-0123456789";

type Row = {
  surface: string;
  source: "workspace" | "instance" | null;
  account: string | null;
  connected_at: string | null;
  inbound: { url: string; secret_set: boolean } | null;
};
const github = (over: Partial<Row> = {}): Row => ({
  surface: "github",
  source: null,
  account: null,
  connected_at: null,
  inbound: { url: "https://maidan.example/workspaces/ws/connections/github/webhook", secret_set: false },
  ...over,
});
const slack = (over: Partial<Row> = {}): Row => ({
  surface: "slack",
  source: null,
  account: null,
  connected_at: null,
  inbound: { url: "https://maidan.example/workspaces/ws/connections/slack/events", secret_set: false },
  ...over,
});
const linear = (over: Partial<Row> = {}): Row => ({ surface: "linear", source: null, account: null, connected_at: null, inbound: null, ...over });

beforeEach(() => notices.clear());
afterEach(() => {
  cleanup();
  window.location.hash = "";
  vi.unstubAllGlobals();
});

/** A router that also keeps each write's body. */
function recording(routes: Route[]) {
  const router = withSession(routes);
  const bodies: Array<{ method: string; url: string; body: string }> = [];
  const fn: typeof router.fn = async (url, init) => {
    if (init?.method && init.method !== "GET") bodies.push({ method: init.method, url: String(url), body: String(init.body ?? "") });
    const res = await router.fn(url, init);
    // A 204 has no body, which a Response with one refuses to be.
    return res.status === 204 ? new Response(null, { status: 204 }) : res;
  };
  return { router, fn, bodies };
}

const rowOf = (surface: string) => document.querySelector<HTMLElement>(`[data-surface="${surface}"]`) as HTMLElement;

test("the three rows say whose account each runs on", async () => {
  const router = withSession([
    {
      match: LIST,
      body: () => [
        github({ source: "workspace", account: "octocat", connected_at: "2026-11-23T16:00:00Z", inbound: { url: "https://maidan.example/workspaces/ws/connections/github/webhook", secret_set: true } }),
        slack({ source: "instance" }),
        linear(),
      ],
    },
  ]);
  await renderSignedIn(<Connections ws="ws" />, router.fn);
  await screen.findByRole("heading", { name: "GitHub" });
  expect(screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent)).toEqual(["GitHub", "Slack", "Linear"]);
  const gh = within(rowOf("github"));
  gh.getByText("Connected");
  gh.getByText(/Connected with this workspace’s own GitHub account, octocat, since .*2026\./);
  expect(gh.getByRole("button", { name: "Remove the GitHub connection…" })).toBeTruthy();
  gh.getByRole("button", { name: "Replace the GitHub connection" });
  const sl = within(rowOf("slack"));
  sl.getByText("Server’s account");
  sl.getByText(/^Using this server’s own Slack connection\./);
  expect(sl.queryByRole("button", { name: /Remove/ })).toBeNull();
  sl.getByRole("button", { name: "Connect Slack" });
  const li = within(rowOf("linear"));
  li.getByText("Not connected");
  li.getByText(/^Not connected\. Nothing in this workspace can send to or read from Linear/);
  expect(li.queryByRole("button", { name: /Remove/ })).toBeNull();
  // Linear has no inbound URL and no inbound secret.
  expect(li.queryByText("Inbound URL")).toBeNull();
  expect(li.queryByLabelText(/secret/i)).toBeNull();
});

test("the token field is a password field, and the connect button waits for a token", async () => {
  const router = withSession([{ match: LIST, body: () => [github(), slack(), linear()] }]);
  await renderSignedIn(<Connections ws="ws" />, router.fn);
  const input = (await within(await waitRow("github")).findByLabelText("Fine-grained personal access token")) as HTMLInputElement;
  expect(input.type).toBe("password");
  expect((within(rowOf("github")).getByLabelText("Webhook secret (optional)") as HTMLInputElement).type).toBe("password");
  expect((within(rowOf("slack")).getByLabelText("Signing secret (optional)") as HTMLInputElement).type).toBe("password");
  expect((within(rowOf("linear")).getByLabelText("API key") as HTMLInputElement).type).toBe("password");
  expect((within(rowOf("github")).getByRole("button", { name: "Connect GitHub" }) as HTMLButtonElement).disabled).toBe(true);
});

async function waitRow(surface: string): Promise<HTMLElement> {
  await waitFor(() => expect(rowOf(surface)).toBeTruthy());
  return rowOf(surface);
}

test("connecting sends the token once, clears the field, and the token is nowhere on the page after", async () => {
  const user = userEvent.setup();
  let connected = false;
  const { router, fn, bodies } = recording([
    { match: LIST, body: () => [connected ? github({ source: "workspace", account: "octocat", connected_at: "2026-11-23T16:00:00Z" }) : github(), slack(), linear()] },
    {
      method: "PUT",
      match: ONE,
      body: () => {
        connected = true;
        return github({ source: "workspace", account: "octocat", connected_at: "2026-11-23T16:00:00Z" });
      },
    },
  ]);
  await renderSignedIn(<Connections ws="ws" />, fn);
  const gh = within(await waitRow("github"));
  await user.type(gh.getByLabelText("Fine-grained personal access token"), `  ${TOKEN} `);
  await user.type(gh.getByLabelText("Webhook secret (optional)"), SECRET);
  await user.click(gh.getByRole("button", { name: "Connect GitHub" }));
  await gh.findByText(/Connected with this workspace’s own GitHub account, octocat/);
  expect(bodies).toEqual([{ method: "PUT", url: "https://console.test/ui/api/workspaces/ws/connections/github", body: JSON.stringify({ token: TOKEN, webhook_secret: SECRET }) }]);
  expect(router.calls.filter((c) => LIST.test(c.url)).length).toBe(2);
  // The fields are empty and the secrets are in no node, attribute or notice.
  expect((gh.getByLabelText("Replace the Fine-grained personal access token") as HTMLInputElement).value).toBe("");
  expect((gh.getByLabelText("Webhook secret (optional)") as HTMLInputElement).value).toBe("");
  expect(document.body.innerHTML).not.toContain(TOKEN);
  expect(document.body.innerHTML).not.toContain(SECRET);
  expect(notices.snapshot().map((n) => n.text)).toEqual(["GitHub connected as octocat"]);
  // Focus lands on the row, which says what changed.
  await waitFor(() => expect(document.activeElement).toBe(rowOf("github")));
});

test("a refused token shows the server's sentence beside the form, and the token is gone from the page", async () => {
  const user = userEvent.setup();
  const { fn } = recording([
    { match: LIST, body: () => [github(), slack(), linear()] },
    // A provider that quotes the token back must not put it on the page.
    { method: "PUT", match: ONE, status: 400, body: () => ({ detail: `GitHub refused this token (${TOKEN}): Bad credentials` }) },
  ]);
  await renderSignedIn(<Connections ws="ws" />, fn);
  const gh = within(await waitRow("github"));
  const input = gh.getByLabelText("Fine-grained personal access token") as HTMLInputElement;
  await user.type(input, TOKEN);
  await user.click(gh.getByRole("button", { name: "Connect GitHub" }));
  const alert = await gh.findByRole("alert");
  expect(alert.textContent).toBe("GitHub refused this token ([hidden]): Bad credentials");
  expect(input.getAttribute("aria-invalid")).toBe("true");
  expect(input.getAttribute("aria-describedby")).toBe(alert.id);
  expect(input.value).toBe("");
  expect(document.body.innerHTML).not.toContain(TOKEN);
  // Focus goes back to the field to try again, and typing clears the complaint.
  await waitFor(() => expect(document.activeElement).toBe(input));
  await user.type(input, "x");
  expect(gh.queryByRole("alert")).toBeNull();
  expect(input.getAttribute("aria-invalid")).toBe("false");
  // The row is still not connected.
  gh.getByText("Not connected");
});

test("a webhook secret that is too short is refused before anything is sent", async () => {
  const user = userEvent.setup();
  const { fn, bodies } = recording([{ match: LIST, body: () => [github(), slack(), linear()] }]);
  await renderSignedIn(<Connections ws="ws" />, fn);
  const gh = within(await waitRow("github"));
  await user.type(gh.getByLabelText("Fine-grained personal access token"), TOKEN);
  await user.type(gh.getByLabelText("Webhook secret (optional)"), "short");
  await user.click(gh.getByRole("button", { name: "Connect GitHub" }));
  expect((await gh.findByRole("alert")).textContent).toBe("A webhook secret is 16 to 128 characters with no spaces.");
  expect(bodies).toEqual([]);
});

test("a connected surface takes a new inbound secret alone and keeps its token", async () => {
  const user = userEvent.setup();
  const own = github({ source: "workspace", account: "octocat", connected_at: "2026-11-23T16:00:00Z" });
  const { fn, bodies } = recording([
    { match: LIST, body: () => [own, slack(), linear()] },
    { method: "PUT", match: ONE, body: () => ({ ...own, inbound: { ...own.inbound, secret_set: true } }) },
  ]);
  await renderSignedIn(<Connections ws="ws" />, fn);
  const gh = within(await waitRow("github"));
  const button = gh.getByRole("button", { name: "Replace the GitHub connection" }) as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  await user.type(gh.getByLabelText("Webhook secret (optional)"), SECRET);
  expect(button.disabled).toBe(false);
  await user.click(button);
  await waitFor(() => expect(bodies.length).toBe(1));
  expect(JSON.parse(bodies[0].body)).toEqual({ webhook_secret: SECRET });
});

test("removing asks first, then deletes, and focus goes to the safe choice and back", async () => {
  const user = userEvent.setup();
  let removed = false;
  const { fn, bodies } = recording([
    { match: LIST, body: () => [removed ? github() : github({ source: "workspace", account: "octocat", connected_at: "2026-11-23T16:00:00Z" }), slack(), linear()] },
    {
      method: "DELETE",
      match: ONE,
      status: 204,
      body: () => {
        removed = true;
        return null;
      },
    },
  ]);
  await renderSignedIn(<Connections ws="ws" />, fn);
  const gh = within(await waitRow("github"));
  await user.click(gh.getByRole("button", { name: "Remove the GitHub connection…" }));
  expect(bodies).toEqual([]);
  // Keep is where focus lands, so a stray Enter cannot remove anything.
  await waitFor(() => expect(document.activeElement).toBe(gh.getByRole("button", { name: "Keep" })));
  await user.click(gh.getByRole("button", { name: "Keep" }));
  await waitFor(() => expect(document.activeElement).toBe(gh.getByRole("button", { name: "Remove the GitHub connection…" })));
  expect(bodies).toEqual([]);
  await user.click(gh.getByRole("button", { name: "Remove the GitHub connection…" }));
  await user.click(gh.getByRole("button", { name: "Remove" }));
  await gh.findByText("Not connected");
  expect(bodies.map((b) => [b.method, b.url])).toEqual([["DELETE", "https://console.test/ui/api/workspaces/ws/connections/github"]]);
  expect(gh.queryByRole("button", { name: /Remove/ })).toBeNull();
  expect(notices.snapshot().map((n) => n.text)).toEqual(["GitHub disconnected"]);
  await waitFor(() => expect(document.activeElement).toBe(rowOf("github")));
});

test("a failed removal says so on the row and leaves the connection", async () => {
  const user = userEvent.setup();
  const own = github({ source: "workspace", account: "octocat", connected_at: "2026-11-23T16:00:00Z" });
  const { fn } = recording([
    { match: LIST, body: () => [own, slack(), linear()] },
    { method: "DELETE", match: ONE, status: 500, body: () => ({ detail: "boom" }) },
  ]);
  await renderSignedIn(<Connections ws="ws" />, fn);
  const gh = within(await waitRow("github"));
  await user.click(gh.getByRole("button", { name: "Remove the GitHub connection…" }));
  await user.click(gh.getByRole("button", { name: "Remove" }));
  expect((await gh.findByRole("alert")).textContent).toMatch(/The server hit an error/);
  gh.getByText(/Connected with this workspace’s own GitHub account/);
});

test("GitHub and Slack show their inbound URL to copy, where to paste it, and whether a secret is set", async () => {
  const user = userEvent.setup();
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
  const router = withSession([
    {
      match: LIST,
      body: () => [github({ source: "workspace", account: "octocat", connected_at: "2026-11-23T16:00:00Z", inbound: { url: "https://maidan.example/workspaces/ws/connections/github/webhook", secret_set: true } }), slack(), linear()],
    },
  ]);
  await renderSignedIn(<Connections ws="ws" />, router.fn);
  const gh = within(await waitRow("github"));
  gh.getByText("https://maidan.example/workspaces/ws/connections/github/webhook");
  gh.getByText("Paste this URL into the repository’s webhook settings as the payload URL, and give the webhook the same secret.");
  gh.getByText("A webhook secret is set. Deliveries to this URL are checked against it.");
  await user.click(gh.getByRole("button", { name: "Copy the GitHub inbound URL" }));
  expect(writeText).toHaveBeenCalledWith("https://maidan.example/workspaces/ws/connections/github/webhook");
  await gh.findByText("Copied", { selector: "button" });
  const sl = within(rowOf("slack"));
  sl.getByText("https://maidan.example/workspaces/ws/connections/slack/events");
  sl.getByText("Paste this URL into the Slack app’s Event Subscriptions as the request URL.");
  sl.getByText("No signing secret is set, so this URL accepts nothing yet. Add one below.");
});

test("a browser that will not copy says so", async () => {
  const user = userEvent.setup();
  vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) } });
  const router = withSession([{ match: LIST, body: () => [github(), slack(), linear()] }]);
  await renderSignedIn(<Connections ws="ws" />, router.fn);
  const gh = within(await waitRow("github"));
  await user.click(gh.getByRole("button", { name: "Copy the GitHub inbound URL" }));
  expect((await gh.findByRole("alert")).textContent).toMatch(/would not copy/);
});

test("anyone who is not a workspace admin is told so, as the allowlist tells them", async () => {
  const router = withSession([{ match: LIST, status: 403, body: () => ({ detail: "only a workspace admin" }) }]);
  const view = await renderSignedIn(<Connections ws="ws" />, router.fn);
  await screen.findByText("Only a workspace admin sees and sets this workspace’s connections. Ask one to connect it.");
  expect(view.container.querySelector("form")).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
});

test("a failed read says so and offers another try", async () => {
  const user = userEvent.setup();
  let fail = true;
  const router = withSession([{ match: LIST, body: () => (fail ? { detail: "database unavailable" } : [github(), slack(), linear()]), status: 200 }]);
  const fn: typeof router.fn = async (url, init) => {
    if (fail && LIST.test(String(url))) return new Response(JSON.stringify({ detail: "x" }), { status: 500, headers: { "content-type": "application/json" } });
    return router.fn(url, init);
  };
  await renderSignedIn(<Connections ws="ws" />, fn);
  expect((await screen.findByRole("alert")).textContent).toMatch(/The server hit an error/);
  fail = false;
  await user.click(screen.getByRole("button", { name: "Try again" }));
  await screen.findByRole("heading", { name: "GitHub" });
});

test("a token with no token:admin is told so without a request, and the Connections tab is in Tools", async () => {
  window.location.hash = "#/tools/connections";
  const router = new FetchRouter([
    { match: /\/auth\/session\/from-token$/, status: 404, body: () => ({}) },
    { match: /\/auth\/session$/, body: () => ({ member_id: "me", workspace_id: "ws", display_name: "Ada" }) },
    { match: /\/me$/, body: () => ({ member_id: "me", workspace_id: "ws", is_bearer: true, capabilities: ["workspace:read"], known_capabilities: ["workspace:read", "token:admin"] }) },
    { match: LIST, body: () => [github()] },
  ]);
  const store = new SessionStore("https://console.test", router.fn);
  globalThis.fetch = router.fn;
  await store.signInWithToken("a-token-without-admin");
  render(
    <SessionContext.Provider value={store}>
      <Tools />
    </SessionContext.Provider>,
  );
  expect(screen.getByRole("tab", { name: "Connections" }).getAttribute("aria-selected")).toBe("true");
  await screen.findByText(/Only a workspace admin sees and sets/);
  expect(router.calls.some((c) => LIST.test(c.url))).toBe(false);
});
