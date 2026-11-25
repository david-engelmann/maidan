import { renderSignedIn, withSession, type Route } from "./console-harness.tsx";
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test } from "vitest";
import { DecisionModel } from "../src/tools/DecisionModel.tsx";

afterEach(() => cleanup());

const ADVISOR = /\/workspaces\/ws\/land-gate\/advisor(\?provider=\w+)?$/;
type Source = "workspace" | "instance" | undefined;
// What the server answers: the default provider's fields, and every provider it can ask.
const three = (typesafe?: Source, openai?: Source, google?: Source) => ({
  enabled: true,
  key_source: typesafe ?? openai ?? google,
  provider: typesafe ? "typesafe" : openai ? "openai" : google ? "google" : "typesafe",
  model: "jev-latest",
  host: "api.typesafe.ai",
  providers: [
    { id: "typesafe", name: "TypeSafe Jev", model: "jev-latest", host: "api.typesafe.ai", key_source: typesafe },
    { id: "openai", name: "OpenAI Decisions", model: "gpt-6-luna", host: "api.openai.com", key_source: openai },
    { id: "google", name: "Gemini Decisions", model: "gemini-decision-1", host: "generativelanguage.googleapis.com", key_source: google },
  ],
});
const on = (key_source?: Source) => three(key_source);
const read = (body: () => unknown, status = 200): Route => ({ method: "GET", match: ADVISOR, body, status });

test("a server that offers no advice shows no panel", async () => {
  for (const route of [read(() => ({ enabled: false })), read(() => ({}), 404)]) {
    const router = withSession([route]);
    const view = await renderSignedIn(<DecisionModel ws="ws" />, router.fn);
    await waitFor(() => expect(router.calls.some((c) => ADVISOR.test(c.url))).toBe(true));
    await waitFor(() => expect(view.container.querySelector(".tool-panel")).toBeNull());
    cleanup();
  }
});

test("the panel says what is sent and when, and saving a key turns advice on", async () => {
  const user = userEvent.setup();
  let stored = false;
  const sent: string[] = [];
  const router = withSession([
    read(() => on(stored ? "workspace" : undefined)),
    {
      method: "PUT",
      match: ADVISOR,
      body: () => {
        stored = true;
        return on("workspace");
      },
    },
  ]);
  const fn: typeof router.fn = (url, init) => {
    if (init?.method === "PUT") sent.push(String(init.body));
    return router.fn(url, init);
  };
  await renderSignedIn(<DecisionModel ws="ws" />, fn);
  expect((await screen.findAllByText(/Off for this workspace until it has a key\./)).length).toBe(3);
  screen.getByText(/Until then nothing is sent\./);
  screen.getByText(/To the provider asked, and no other/);
  screen.getByText(/the task’s title, description and acceptance, its result, and the messages of the agent that did the work/);
  screen.getByText(/The answers show on the task as advice for whoever reviews it\./);
  const input = screen.getByLabelText("Your TypeSafe API key") as HTMLInputElement;
  expect(input.type).toBe("password");
  const save = screen.getByRole("button", { name: "Save the TypeSafe API key" }) as HTMLButtonElement;
  expect(save.disabled).toBe(true);
  await user.type(input, "  ts_live_fixture ");
  await user.click(save);
  await screen.findByText(/On, with this workspace’s own key\./);
  expect(sent).toEqual([JSON.stringify({ api_key: "ts_live_fixture", provider: "typesafe" })]);
  // The key is gone from the page once it is saved.
  expect((screen.getByLabelText("Replace the TypeSafe API key") as HTMLInputElement).value).toBe("");
});

test("removing the key asks first", async () => {
  const user = userEvent.setup();
  let stored = true;
  const router = withSession([
    read(() => on(stored ? "workspace" : undefined)),
    {
      method: "DELETE",
      match: ADVISOR,
      body: () => {
        stored = false;
        return on(undefined);
      },
    },
  ]);
  await renderSignedIn(<DecisionModel ws="ws" />, router.fn);
  await user.click(await screen.findByRole("button", { name: "Remove the TypeSafe API key…" }));
  expect(router.calls.some((c) => c.method === "DELETE")).toBe(false);
  await user.click(screen.getByRole("button", { name: "Remove" }));
  await waitFor(() => expect(screen.getAllByText(/Off for this workspace until it has a key\./).length).toBe(3));
  const removed = router.calls.filter((c) => c.method === "DELETE");
  expect(removed.length).toBe(1);
  expect(removed[0].url).toMatch(/\?provider=typesafe$/);
});

test("a failed read says so and offers another try, where a hidden panel would leave nothing to retry", async () => {
  const user = userEvent.setup();
  let fail = true;
  const router = withSession([read(() => (fail ? { detail: "database unavailable" } : on("instance")), 500)]);
  // The status flips to 200 once the read is allowed to pass.
  const fn: typeof router.fn = async (url, init) => {
    if (!fail && ADVISOR.test(String(url))) return new Response(JSON.stringify(on("instance")), { status: 200, headers: { "content-type": "application/json" } });
    return router.fn(url, init);
  };
  await renderSignedIn(<DecisionModel ws="ws" />, fn);
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toMatch(/Could not read how advice is set up here/);
  fail = false;
  await user.click(screen.getByRole("button", { name: "Try again" }));
  await screen.findByText(/On, with this server’s key\./);
  expect(screen.queryByRole("alert")).toBeNull();
});

test("each provider has its own key, saved to that provider and no other", async () => {
  const user = userEvent.setup();
  let google: Source;
  const sent: string[] = [];
  const router = withSession([
    read(() => three("instance", undefined, google)),
    {
      method: "PUT",
      match: ADVISOR,
      body: () => {
        google = "workspace";
        return three("instance", undefined, google);
      },
    },
  ]);
  const fn: typeof router.fn = (url, init) => {
    if (init?.method === "PUT") sent.push(String(init.body));
    return router.fn(url, init);
  };
  await renderSignedIn(<DecisionModel ws="ws" />, fn);
  // Three providers, each with where it sends to and whose key it would use.
  await screen.findByRole("heading", { name: /TypeSafe Jev/ });
  screen.getByRole("heading", { name: /OpenAI Decisions/ });
  screen.getByRole("heading", { name: /Gemini Decisions/ });
  screen.getByText(/On, with this server’s key\./);
  screen.getByText(/Sent to generativelanguage\.googleapis\.com, model gemini-decision-1\./);
  expect(screen.getAllByText(/Off for this workspace until it has a key\./).length).toBe(2);

  await user.type(screen.getByLabelText("Your Gemini API key"), "g-fixture");
  await user.click(screen.getByRole("button", { name: "Save the Gemini API key" }));
  await screen.findByText(/On, with this workspace’s own key\./);
  expect(sent).toEqual([JSON.stringify({ api_key: "g-fixture", provider: "google" })]);
  // The other providers' fields are untouched.
  expect((screen.getByLabelText("Your OpenAI API key") as HTMLInputElement).value).toBe("");
});

test("Cloudflare's token is saved with the account it is for", async () => {
  const user = userEvent.setup();
  let source: Source;
  const sent: string[] = [];
  const status = () => ({
    enabled: true,
    key_source: source,
    provider: "cloudflare",
    model: "clef",
    host: "api.cloudflare.com",
    providers: [
      { id: "perplexity", name: "Perplexity Decider", model: "pplx-decider-latest", host: "api.perplexity.ai" },
      { id: "cloudflare", name: "Cloudflare Clef", model: "clef", host: "api.cloudflare.com", key_source: source },
    ],
  });
  const router = withSession([
    read(status),
    {
      method: "PUT",
      match: ADVISOR,
      body: () => {
        source = "workspace";
        return status();
      },
    },
  ]);
  const fn: typeof router.fn = (url, init) => {
    if (init?.method === "PUT") sent.push(String(init.body));
    return router.fn(url, init);
  };
  await renderSignedIn(<DecisionModel ws="ws" />, fn);
  await screen.findByRole("heading", { name: /Cloudflare Clef/ });
  screen.getByText(/Sent to api\.cloudflare\.com, model clef\./);
  // Only Cloudflare asks for an account.
  expect(screen.getAllByLabelText("Cloudflare account ID").length).toBe(1);
  const save = screen.getByRole("button", { name: "Save the Cloudflare Workers AI token" });
  await user.type(screen.getByLabelText("Your Cloudflare Workers AI token"), "cf-token");
  expect((save as HTMLButtonElement).disabled, "a token with no account is not sent").toBe(true);
  await user.type(screen.getByLabelText("Cloudflare account ID"), " 0123456789abcdef0123456789abcdef ");
  await user.click(save);
  await screen.findByText(/On, with this workspace’s own key\./);
  expect(sent).toEqual([JSON.stringify({ api_key: "cf-token", provider: "cloudflare", account_id: "0123456789abcdef0123456789abcdef" })]);
  // Perplexity's key is the key alone.
  await user.type(screen.getByLabelText("Your Perplexity API key"), "pplx-fixture");
  await user.click(screen.getByRole("button", { name: "Save the Perplexity API key" }));
  await waitFor(() => expect(sent.length).toBe(2));
  expect(sent[1]).toBe(JSON.stringify({ api_key: "pplx-fixture", provider: "perplexity" }));
});

test("a server with no encryption key says so in place of the forms", async () => {
  const router = withSession([read(() => ({ ...three(), can_store_key: false }))]);
  await renderSignedIn(<DecisionModel ws="ws" />, router.fn);
  await screen.findByText("This server cannot store a decision model key. Its operator has not set an encryption key.");
  expect(screen.queryByLabelText(/^Your .* (API key|token)$/)).toBeNull();
  expect(screen.queryByRole("button", { name: /^Save the/ })).toBeNull();
  expect(screen.queryByText(/Saving sends it to its provider once/)).toBeNull();
  // The providers are still listed, with where each stands.
  expect((screen.getAllByText(/Off for this workspace until it has a key\./)).length).toBe(3);
});

test("a server that can store a key, or does not say, offers the forms", async () => {
  for (const body of [{ ...three(), can_store_key: true }, three()]) {
    const router = withSession([read(() => body)]);
    await renderSignedIn(<DecisionModel ws="ws" />, router.fn);
    await screen.findByLabelText("Your TypeSafe API key");
    expect(screen.queryByText(/cannot store a decision model key/)).toBeNull();
    cleanup();
  }
});
