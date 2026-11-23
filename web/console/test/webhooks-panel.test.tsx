import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test } from "vitest";
import { notices } from "../src/core/notices.ts";
import { SessionContext } from "../src/hooks/useSession.ts";
import { SessionStore } from "../src/core/session.ts";
import { Tools } from "../src/tools/Tools.tsx";
import { ALL_EVENT_KINDS, EVENT_GROUPS } from "../src/core/eventKinds.ts";
import { Webhooks } from "../src/tools/Webhooks.tsx";
import { FetchRouter, renderSignedIn, withSession, type Route } from "./console-harness.tsx";

const LIST = /\/workspaces\/ws\/webhooks$/;
const ONE = /\/workspaces\/ws\/webhooks\/[\w-]+$/;
const SECRET = "mdn_signing_fixture_0123456789abcdef";

type Hook = { id: string; workspace_id: string; url: string; label: string | null; event_kinds: string[]; enabled: boolean; created_at: string; revoked_at: string | null };
const hook = (over: Partial<Hook> = {}): Hook => ({
  id: "w1",
  workspace_id: "ws",
  url: "https://example.com/maidan",
  label: "primary",
  event_kinds: ["message_posted", "thread_result_set"],
  enabled: true,
  created_at: "2026-11-23T16:00:00Z",
  revoked_at: null,
  ...over,
});

beforeEach(() => notices.clear());
afterEach(() => {
  cleanup();
  window.location.hash = "";
});

/** Signs in with a pasted token, the way the webhook routes need, and records each write's body. */
async function renderWithToken(ui: React.ReactNode, routes: Route[], grant = ["workspace:read", "workspace:write", "workspace:admin"]) {
  const bodies: Array<{ method: string; url: string; body: string; auth: string | null }> = [];
  const router = new FetchRouter([
    { match: /\/auth\/session\/from-token$/, status: 404, body: () => ({}) },
    { match: /\/auth\/session$/, body: () => ({ member_id: "me", workspace_id: "ws", display_name: "Ada" }) },
    { match: /\/me$/, body: () => ({ member_id: "me", workspace_id: "ws", is_bearer: true, capabilities: grant, known_capabilities: grant }) },
    ...routes,
  ]);
  const fn: typeof router.fn = async (url, init) => {
    if (init?.method && init.method !== "GET" && /\/workspaces\//.test(String(url))) bodies.push({ method: init.method, url: String(url), body: String(init.body ?? ""), auth: new Headers(init.headers).get("Authorization") });
    const res = await router.fn(url, init);
    return res.status === 204 ? new Response(null, { status: 204 }) : res;
  };
  const store = new SessionStore("https://console.test", fn);
  globalThis.fetch = fn;
  await store.signInWithToken("a-token");
  const view = render(<SessionContext.Provider value={store}>{ui}</SessionContext.Provider>);
  return { view, router, bodies };
}

/** A webhook's row, once the list has loaded. */
const rowOf = (id: string) =>
  waitFor(() => {
    const el = document.querySelector<HTMLElement>(`[data-webhook="${id}"]`);
    expect(el).toBeTruthy();
    return el as HTMLElement;
  });

const choose = async (user: ReturnType<typeof userEvent.setup>, ...kinds: string[]) => {
  for (const k of kinds) await user.click(screen.getByRole("checkbox", { name: k }));
};

test("each webhook shows its URL, label, events and when it was added", async () => {
  await renderWithToken(<Webhooks ws="ws" />, [{ match: LIST, body: () => [hook(), hook({ id: "w2", label: null, url: "https://example.com/other", event_kinds: ["claim_expired"] })] }]);
  const first = await rowOf("w1");
  const a = within(first);
  a.getByText("primary");
  a.getByText("https://example.com/maidan");
  expect(a.getAllByRole("listitem").map((l) => l.textContent)).toEqual(["Message posted", "Thread result set"]);
  expect(first.querySelector("time")?.getAttribute("datetime")).toBe("2026-11-23T16:00:00Z");
  // A webhook with no label is named by its URL, once.
  const second = within(document.querySelector<HTMLElement>('[data-webhook="w2"]') as HTMLElement);
  expect(second.getAllByText("https://example.com/other").length).toBe(1);
  second.getByText("Claim expired");
});

test("the form says the URL rules, offers the console's event kinds, and waits for a URL and an event", async () => {
  const user = userEvent.setup();
  await renderWithToken(<Webhooks ws="ws" />, [{ match: LIST, body: () => [] }]);
  await screen.findByText(/No webhooks yet/);
  screen.getByText("An absolute http or https URL, with no user name or password in it, at a public network address. Redirects are not followed.");
  const add = screen.getByRole("button", { name: "Add webhook" }) as HTMLButtonElement;
  expect(add.disabled).toBe(true);
  await user.type(screen.getByLabelText("URL"), "https://example.com/hook");
  expect(add.disabled, "a URL alone is not enough").toBe(true);
  await choose(user, "Message posted");
  expect(add.disabled).toBe(false);
  await user.click(screen.getByRole("checkbox", { name: "Message posted" }));
  expect(add.disabled).toBe(true);
});

test("adding sends the URL, label and chosen events, then shows the signing secret once", async () => {
  const user = userEvent.setup();
  let made = false;
  const { bodies } = await renderWithToken(<Webhooks ws="ws" />, [
    { match: LIST, body: () => (made ? [hook()] : []) },
    {
      method: "POST",
      match: LIST,
      status: 201,
      body: () => {
        made = true;
        return { webhook: hook(), secret: SECRET };
      },
    },
  ]);
  await screen.findByText(/No webhooks yet/);
  await user.type(screen.getByLabelText("URL"), " https://example.com/maidan ");
  await user.type(screen.getByLabelText("Label (optional)"), "primary");
  await choose(user, "Thread result set", "Message posted");
  await user.click(screen.getByRole("button", { name: "Add webhook" }));
  const box = await screen.findByRole("region", { name: "Signing secret for primary" });
  expect(bodies).toEqual([{ method: "POST", url: "https://console.test/workspaces/ws/webhooks", body: JSON.stringify({ url: "https://example.com/maidan", label: "primary", event_kinds: ["thread_result_set", "message_posted"] }), auth: "Bearer a-token" }]);
  // What the secret is for, in the names the server sends.
  within(box).getByText(/It signs every delivery to this webhook\./);
  within(box).getByText(/X-Maidan-Signature/);
  expect(within(box).queryByText(/keeps only a hash/)).toBeNull();
  // It is hidden until shown, and the form is empty and the list is new.
  expect(document.body.innerHTML).not.toContain(SECRET);
  await user.click(within(box).getByRole("button", { name: "Show" }));
  expect(document.body.innerHTML).toContain(SECRET);
  expect((screen.getByLabelText("URL") as HTMLInputElement).value).toBe("");
  await screen.findByText("primary", { selector: "b" });
  // "I have it" removes it for good, and the page and the notices never held it.
  await user.click(within(box).getByRole("button", { name: "I have it" }));
  expect(screen.queryByRole("region", { name: /Signing secret/ })).toBeNull();
  expect(document.body.innerHTML).not.toContain(SECRET);
  expect(notices.snapshot().map((n) => n.text)).toEqual(["Webhook added for primary"]);
  await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText("URL")));
});

test("a refused URL shows the server's sentence beside the form and keeps what was typed", async () => {
  const user = userEvent.setup();
  const { bodies } = await renderWithToken(<Webhooks ws="ws" />, [
    { match: LIST, body: () => [] },
    { method: "POST", match: LIST, status: 400, body: () => ({ detail: "egress target is not a public network address" }) },
  ]);
  await screen.findByText(/No webhooks yet/);
  const input = screen.getByLabelText("URL") as HTMLInputElement;
  await user.type(input, "http://10.0.0.5/hook");
  await choose(user, "Message posted");
  await user.click(screen.getByRole("button", { name: "Add webhook" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toBe("egress target is not a public network address");
  expect(input.getAttribute("aria-invalid")).toBe("true");
  expect(input.getAttribute("aria-describedby")).toContain(alert.id);
  expect(input.value).toBe("http://10.0.0.5/hook");
  expect(bodies.length).toBe(1);
  expect(screen.queryByRole("region", { name: /Signing secret/ })).toBeNull();
  await waitFor(() => expect(document.activeElement).toBe(input));
  await user.type(input, "x");
  expect(screen.queryByRole("alert")).toBeNull();
  expect(input.getAttribute("aria-invalid")).toBe("false");
});

test("a token that is not an admin gets the server's sentence, and a server that cannot make one says what to check", async () => {
  const user = userEvent.setup();
  let status = 403;
  await renderWithToken(<Webhooks ws="ws" />, [
    { match: LIST, body: () => [] },
    { method: "POST", match: LIST, get status() { return status; }, body: () => (status === 403 ? { detail: "token is missing the workspace:admin capability" } : { detail: "the server failed while handling this request" }) } as Route,
  ], ["workspace:read"]);
  await screen.findByText(/No webhooks yet/);
  await user.type(screen.getByLabelText("URL"), "https://example.com/hook");
  await choose(user, "Message posted");
  await user.click(screen.getByRole("button", { name: "Add webhook" }));
  expect((await screen.findByRole("alert")).textContent).toBe("token is missing the workspace:admin capability");
  status = 500;
  await user.click(screen.getByRole("button", { name: "Add webhook" }));
  await waitFor(() => expect(screen.getByRole("alert").textContent).toMatch(/Creating one needs FEDERATION_ENCRYPTION_KEY/));
});

test("revoking asks first, focus lands on the safe choice, and a revoke deletes that webhook", async () => {
  const user = userEvent.setup();
  let gone = false;
  const { bodies } = await renderWithToken(<Webhooks ws="ws" />, [
    { match: LIST, body: () => (gone ? [] : [hook()]) },
    {
      method: "DELETE",
      match: ONE,
      status: 204,
      body: () => {
        gone = true;
        return null;
      },
    },
  ]);
  const row = within(await rowOf("w1"));
  await user.click(row.getByRole("button", { name: "Revoke the webhook for primary…" }));
  expect(bodies).toEqual([]);
  await waitFor(() => expect(document.activeElement).toBe(row.getByRole("button", { name: "Keep" })));
  await user.click(row.getByRole("button", { name: "Keep" }));
  await waitFor(() => expect(document.activeElement).toBe(row.getByRole("button", { name: "Revoke the webhook for primary…" })));
  expect(bodies).toEqual([]);
  await user.click(row.getByRole("button", { name: "Revoke the webhook for primary…" }));
  await user.click(row.getByRole("button", { name: "Revoke" }));
  await screen.findByText(/No webhooks yet/);
  expect(bodies.map((b) => [b.method, b.url])).toEqual([["DELETE", "https://console.test/workspaces/ws/webhooks/w1"]]);
  expect(notices.snapshot().map((n) => n.text)).toEqual(["Webhook revoked for primary"]);
  await waitFor(() => expect(document.activeElement?.getAttribute("aria-labelledby")).toBe("webhooks-title"));
});

test("a failed revoke says so by the list and leaves the webhook", async () => {
  const user = userEvent.setup();
  await renderWithToken(<Webhooks ws="ws" />, [
    { match: LIST, body: () => [hook()] },
    { method: "DELETE", match: ONE, status: 403, body: () => ({ detail: "token is missing the workspace:write capability" }) },
  ]);
  const row = within(await rowOf("w1"));
  await user.click(row.getByRole("button", { name: "Revoke the webhook for primary…" }));
  await user.click(row.getByRole("button", { name: "Revoke" }));
  expect((await screen.findByRole("alert")).textContent).toBe("token is missing the workspace:write capability");
  expect(document.querySelector('[data-webhook="w1"]')).toBeTruthy();
  // The form's URL field is not the one complaining.
  expect((screen.getByLabelText("URL") as HTMLInputElement).getAttribute("aria-invalid")).toBe("false");
});

test("a token that cannot list webhooks is told so, and a failed read offers another try", async () => {
  await renderWithToken(<Webhooks ws="ws" />, [{ match: LIST, status: 403, body: () => ({ detail: "no" }) }], ["workspace:admin"]);
  await screen.findByText("This sign-in cannot list webhooks. Listing needs workspace:read.");
  expect(document.querySelector("form")).toBeNull();
  cleanup();
  const user = userEvent.setup();
  let fail = true;
  await renderWithToken(<Webhooks ws="ws" />, [{ match: LIST, get status() { return fail ? 500 : 200; }, body: () => (fail ? { detail: "x" } : []) } as Route]);
  expect((await screen.findByRole("alert")).textContent).toMatch(/The server hit an error/);
  fail = false;
  await user.click(screen.getByRole("button", { name: "Try again" }));
  await screen.findByText(/No webhooks yet/);
});

test("a browser sign-in uses the console twin for the list, the add and the revoke, with no token needed", async () => {
  const user = userEvent.setup();
  let gone = false;
  const bodies: Array<[string, string]> = [];
  const router = withSession([
    { match: LIST, body: () => (gone ? [] : [hook()]) },
    { method: "POST", match: LIST, status: 201, body: () => ({ webhook: hook({ id: "w2" }), secret: SECRET }) },
    { method: "DELETE", match: ONE, status: 204, body: () => { gone = true; return null; } },
  ]);
  const fn: typeof router.fn = async (url, init) => {
    if (init?.method && init.method !== "GET") bodies.push([init.method, String(url)]);
    const res = await router.fn(url, init);
    return res.status === 204 ? new Response(null, { status: 204 }) : res;
  };
  await renderSignedIn(<Webhooks ws="ws" />, fn);
  const row = within(await rowOf("w1"));
  expect(router.calls.find((c) => LIST.test(c.url))?.url).toBe("https://console.test/ui/api/workspaces/ws/webhooks");
  expect(screen.queryByText(/paste it under Tools, Connect/)).toBeNull();
  await user.type(screen.getByLabelText("URL"), "https://example.com/new");
  await choose(user, "Message posted");
  await user.click(screen.getByRole("button", { name: "Add webhook" }));
  await screen.findByRole("region", { name: /Signing secret/ });
  await user.click(row.getByRole("button", { name: "Revoke the webhook for primary…" }));
  await user.click(row.getByRole("button", { name: "Revoke" }));
  await screen.findByText(/No webhooks yet/);
  expect(bodies).toEqual([
    ["POST", "https://console.test/ui/api/workspaces/ws/webhooks"],
    ["DELETE", "https://console.test/ui/api/workspaces/ws/webhooks/w1"],
  ]);
});

test("a server that cannot keep the secret answers 503 and its own sentence is shown", async () => {
  const user = userEvent.setup();
  await renderWithToken(<Webhooks ws="ws" />, [
    { match: LIST, body: () => [] },
    { method: "POST", match: LIST, status: 503, body: () => ({ detail: "Set FEDERATION_ENCRYPTION_KEY on the server to create webhooks" }) },
  ]);
  await screen.findByText(/No webhooks yet/);
  await user.type(screen.getByLabelText("URL"), "https://example.com/hook");
  await choose(user, "Message posted");
  await user.click(screen.getByRole("button", { name: "Add webhook" }));
  expect((await screen.findByRole("alert")).textContent).toBe("Set FEDERATION_ENCRYPTION_KEY on the server to create webhooks");
});

test("every kind the server has is offered in groups, and a whole group can be chosen with one box", async () => {
  const user = userEvent.setup();
  const { bodies } = await renderWithToken(<Webhooks ws="ws" />, [
    { match: LIST, body: () => [] },
    { method: "POST", match: LIST, status: 201, body: () => ({ webhook: hook(), secret: SECRET }) },
  ]);
  await screen.findByText(/No webhooks yet/);
  const form = within(screen.getByRole("form", { name: "Add a webhook" }));
  expect(form.getAllByRole("checkbox").length - EVENT_GROUPS.length).toBe(ALL_EVENT_KINDS.length);
  expect(ALL_EVENT_KINDS.length).toBe(52);
  screen.getByText("Choose at least one. 52 to choose from, in 10 groups.");
  const group = within(document.querySelector<HTMLElement>('[data-group="claims"]') as HTMLElement);
  group.getByText("4 events");
  const all = group.getByRole("checkbox", { name: "All claims events" }) as HTMLInputElement;
  // One kind makes the group box partly on, and the group says so.
  await user.click(group.getByRole("checkbox", { name: "Claim expired" }));
  expect(all.indeterminate).toBe(true);
  expect(all.checked).toBe(false);
  group.getByText("1 of 4 chosen");
  await user.click(all);
  expect(all.indeterminate).toBe(false);
  expect(all.checked).toBe(true);
  group.getByText("4 of 4 chosen");
  screen.getByText("Choose at least one. 4 of 52 chosen.");
  // Keyboard: the box and a kind inside answer the space bar.
  all.focus();
  await user.keyboard(" ");
  expect(all.checked).toBe(false);
  group.getByText("4 events");
  await user.click(all);
  await user.type(screen.getByLabelText("URL"), "https://example.com/hook");
  await user.click(screen.getByRole("button", { name: "Add webhook" }));
  await screen.findByRole("region", { name: /Signing secret/ });
  expect(JSON.parse(bodies[0].body).event_kinds).toEqual(["claim_expired", "claim_unacknowledged", "claim_stalled", "claim_failed"]);
});

test("the Integrations tab holds the accounts, the webhooks and what they sent", async () => {
  window.location.hash = "#/tools/integrations";
  await renderWithToken(<Tools />, [
    { match: /\/workspaces\/ws\/connections$/, body: () => [] },
    { match: LIST, body: () => [] },
    { match: /\/workspaces\/ws\/deliveries/, body: () => [] },
  ]);
  await screen.findByRole("heading", { name: "GitHub, Slack and Linear" });
  await screen.findByRole("heading", { name: "Webhooks" });
  await screen.findByRole("heading", { name: "Webhook and automation sends" });
  // The sends moved here from Deliveries.
  window.location.hash = "#/tools/deliveries";
  await waitFor(() => expect(screen.queryByRole("heading", { name: "Webhook and automation sends" })).toBeNull());
});
