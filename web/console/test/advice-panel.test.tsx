import { renderSignedIn, withSession, type Route } from "./console-harness.tsx";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test } from "vitest";
import { Advice } from "../src/board/Advice.tsx";

afterEach(() => cleanup());

const TASK = "0190a1b2-0000-7000-8000-0000000000aa";
const judgment = (id: string, label: string, probability: number, reading: string, concern = false) => ({
  id,
  label,
  probability,
  answer: probability >= 0.7 ? "yes" : probability <= 0.3 ? "no" : "unsettled",
  reading,
  concern,
});
const base = { provider: "typesafe", model: "jev-1.14.0", key_source: "instance", cached: false };
const green = {
  ...base,
  land: "green",
  reasons: [],
  judgments: [
    judgment("done", "Does what the task asks", 0.95, "Does what the task asks"),
    judgment("unresolved_failure", "Something is still failing", 0.02, "Reports nothing still failing"),
  ],
};
const amber = {
  ...base,
  land: "amber",
  reasons: ["tests_not_reported", "flags_risk"],
  judgments: [
    judgment("done", "Does what the task asks", 0.95, "Does what the task asks"),
    judgment("tests_passed", "Reports passing tests", 0.02, "Reports no passing tests", true),
    judgment("code_change", "Asks for a code change", 0.98, "The task asks for a code change"),
    judgment("flags_risk", "Warns about a risk", 0.5, "Unclear whether the agent flags a risk", true),
  ],
};

const advice = (body: () => unknown, status = 200): Route => ({ method: "POST", match: /\/threads\/[^/]+\/land-gate\/advice$/, body, status });

test("a clear hand-off says nothing was flagged, that it is advice only, and keeps the judgments one click away", async () => {
  const user = userEvent.setup();
  const router = withSession([advice(() => green)]);
  await renderSignedIn(<Advice tid={TASK} />, router.fn);
  const panel = await screen.findByRole("region", { name: "Advice" });
  await within(panel).findByText("Nothing flagged");
  expect(panel.getAttribute("data-land")).toBe("green");
  within(panel).getByText("All ten readings are clear.");
  within(panel).getByText("jev-1.14.0");
  within(panel).getByText(/Advice only\./);
  expect(panel.querySelector("details")?.open).toBe(false);
  await user.click(within(panel).getByText("All ten readings"));
  // Each judgment is a statement with how sure the model is of it: 2% yes
  // reads as 98% sure of the no.
  expect(within(panel).getByLabelText("Does what the task asks, 95% sure")).toBeTruthy();
  expect(within(panel).getByLabelText("Reports nothing still failing, 98% sure")).toBeTruthy();
});

test("what kept the advice from green is said in words, and its rows are marked where they stand", async () => {
  const router = withSession([advice(() => amber)]);
  await renderSignedIn(<Advice tid={TASK} />, router.fn);
  const panel = await screen.findByRole("region", { name: "Advice" });
  await within(panel).findByText("Worth a look");
  within(panel).getByText("The task asks for a code change, and no passing tests are reported.");
  within(panel).getByText("It carries a warning from the agent about a risk.");
  const rows = Array.from(panel.querySelectorAll(".advice-row"));
  // The server's order is kept, so a row is where it was the last time.
  expect(rows.map((li) => [li.getAttribute("aria-label"), li.getAttribute("data-mark")])).toEqual([
    ["Does what the task asks, 95% sure", "clear"],
    ["Flagged: Reports no passing tests, 98% sure", "concern"],
    ["The task asks for a code change, 98% sure", "clear"],
    ["Unsure: Unclear whether the agent flags a risk, unsure", "unsure"],
  ]);
});

test("Ask again asks the model afresh", async () => {
  const user = userEvent.setup();
  let asked = 0;
  const bodies: string[] = [];
  const router = withSession([advice(() => (++asked === 1 ? amber : green))]);
  const fn: typeof router.fn = (url, init) => {
    if (String(url).endsWith("/land-gate/advice")) bodies.push(String(init?.body));
    return router.fn(url, init);
  };
  await renderSignedIn(<Advice tid={TASK} />, fn);
  await screen.findByText("Worth a look");
  await user.click(screen.getByRole("button", { name: "Ask again" }));
  await screen.findByText("Nothing flagged");
  expect(bodies).toEqual(["{}", JSON.stringify({ refresh: true })]);
});

test("with advice off, or no result to read, the sheet shows nothing", async () => {
  for (const route of [advice(() => ({}), 404), advice(() => ({ detail: "this task has no result yet" }), 409)]) {
    const router = withSession([route]);
    const view = await renderSignedIn(<Advice tid={TASK} />, router.fn);
    await waitFor(() => expect(router.calls.some((c) => c.url.endsWith("/land-gate/advice"))).toBe(true));
    await waitFor(() => expect(view.container.querySelector(".advice")).toBeNull());
    cleanup();
  }
});

test("a workspace with no key is told where to add one, and a failure offers another try", async () => {
  const noKey = withSession([advice(() => ({ detail: "this workspace has no TypeSafe key: store one" }), 409)]);
  await renderSignedIn(<Advice tid={TASK} />, noKey.fn);
  const link = await screen.findByRole("link", { name: "Add a key for one under Tools, Workspace." });
  // The key panel is on the Workspace tab, not the tab Tools opens on.
  expect(link.getAttribute("href")).toBe("#/tools/workspace");
  cleanup();
  const down = withSession([advice(() => ({ detail: "land-gate advisor unavailable" }), 502)]);
  await renderSignedIn(<Advice tid={TASK} />, down.fn);
  await screen.findByText(/The task is unchanged/);
  screen.getByRole("button", { name: "Ask again" });
});

function Changing() {
  const [stamp, setStamp] = useState("2026-11-20T10:00:00Z");
  return (
    <>
      <button type="button" onClick={() => setStamp("2026-11-20T10:05:00Z")}>The task changed</button>
      <Advice tid={TASK} stamp={stamp} />
    </>
  );
}

test("a task that changes while its sheet is open is asked about again, without forcing the model", async () => {
  const user = userEvent.setup();
  let asked = 0;
  const bodies: string[] = [];
  const router = withSession([advice(() => (++asked === 1 ? amber : green))]);
  const fn: typeof router.fn = (url, init) => {
    if (String(url).endsWith("/land-gate/advice")) bodies.push(String(init?.body));
    return router.fn(url, init);
  };
  await renderSignedIn(<Changing />, fn);
  await screen.findByText("Worth a look");
  await user.click(screen.getByRole("button", { name: "The task changed" }));
  // The answer for the earlier hand-off gives way to the one for this one.
  await screen.findByText("Nothing flagged");
  expect(screen.queryByText("Worth a look")).toBeNull();
  // Not a refresh: the server answers from what it holds when the hand-off is the same.
  expect(bodies).toEqual(["{}", "{}"]);
});

test("a task that changes while Ask again is in flight still gets a fresh answer", async () => {
  const user = userEvent.setup();
  const bodies: string[] = [];
  let release: () => void = () => {};
  let asked = 0;
  const router = withSession([
    advice(() => {
      asked += 1;
      if (asked === 1) return amber;
      // The answer to Ask again is held back until after the task has changed.
      if (asked === 2) return new Promise((resolve) => (release = () => resolve(amber)));
      return green;
    }),
  ]);
  const fn: typeof router.fn = (url, init) => {
    if (String(url).endsWith("/land-gate/advice")) bodies.push(String(init?.body));
    return router.fn(url, init);
  };
  await renderSignedIn(<Changing />, fn);
  await screen.findByText("Worth a look");
  await user.click(screen.getByRole("button", { name: "Ask again" }));
  await user.click(screen.getByRole("button", { name: "The task changed" }));
  await screen.findByText("Nothing flagged");
  // The request for the changed task asks afresh too: a held answer must not
  // come back in the fresh one's place.
  expect(bodies).toEqual(["{}", JSON.stringify({ refresh: true }), JSON.stringify({ refresh: true })]);
  release();
  // The late answer to the first Ask again is not shown over the newer one.
  await waitFor(() => expect(screen.queryByText("Worth a look")).toBeNull());
});

test("with keys for more than one provider the reviewer can switch who is asked, and the choice is kept", async () => {
  const user = userEvent.setup();
  window.localStorage.removeItem("maidan_advice_provider");
  const asked: string[] = [];
  const providers = [
    { id: "typesafe", name: "TypeSafe Jev", model: "jev-latest", host: "api.typesafe.ai", key_source: "workspace" },
    { id: "openai", name: "OpenAI Decisions", model: "gpt-6-luna", host: "api.openai.com" },
    { id: "google", name: "Gemini Decisions", model: "gemini-decision-1", host: "generativelanguage.googleapis.com", key_source: "instance" },
  ];
  const router = withSession([
    { match: /\/workspaces\/ws\/land-gate\/advisor$/, body: () => ({ enabled: true, key_source: "workspace", provider: "typesafe", model: "jev-latest", host: "api.typesafe.ai", providers }) },
    advice(() => (asked.at(-1)?.includes("google") ? { ...amber, provider: "google", model: "gemini-decision-1" } : green)),
  ]);
  const fn: typeof router.fn = (url, init) => {
    if (init?.method === "POST") asked.push(String(init.body));
    return router.fn(url, init);
  };
  await renderSignedIn(<Advice tid={TASK} />, fn);
  const panel = await screen.findByRole("region", { name: "Advice" });
  await within(panel).findByText("Nothing flagged");
  // Only the providers with a key are offered.
  const pick = (await within(panel).findByLabelText("Decision model")) as HTMLSelectElement;
  expect([...pick.options].map((o) => o.text)).toEqual(["TypeSafe Jev", "Gemini Decisions"]);
  expect(asked[0]).toBe("{}");

  await user.selectOptions(pick, "google");
  await within(panel).findByText("Worth a look");
  within(panel).getByText("gemini-decision-1");
  expect(asked.at(-1)).toBe(JSON.stringify({ provider: "google" }));
  expect(window.localStorage.getItem("maidan_advice_provider")).toBe("google");
  window.localStorage.removeItem("maidan_advice_provider");
});

test("one provider with a key shows no switch", async () => {
  const router = withSession([
    { match: /\/workspaces\/ws\/land-gate\/advisor$/, body: () => ({ enabled: true, key_source: "instance", provider: "typesafe", model: "jev-latest", host: "api.typesafe.ai", providers: [{ id: "typesafe", name: "TypeSafe Jev", model: "jev-latest", host: "api.typesafe.ai", key_source: "instance" }, { id: "openai", name: "OpenAI Decisions", model: "gpt-6-luna", host: "api.openai.com" }] }) },
    advice(() => green),
  ]);
  await renderSignedIn(<Advice tid={TASK} />, router.fn);
  const panel = await screen.findByRole("region", { name: "Advice" });
  await within(panel).findByText("Nothing flagged");
  await new Promise((r) => setTimeout(r, 50));
  expect(within(panel).queryByLabelText("Decision model")).toBeNull();
});

test("a reviewer who chose a model is not asked for through another while the choice is being checked", async () => {
  window.localStorage.setItem("maidan_advice_provider", "google");
  const asked: string[] = [];
  let release: () => void = () => undefined;
  const held = new Promise<void>((r) => { release = r; });
  const providers = [
    { id: "typesafe", name: "TypeSafe Jev", model: "jev-latest", host: "api.typesafe.ai", key_source: "workspace" },
    { id: "google", name: "Gemini Decisions", model: "gemini-decision-1", host: "generativelanguage.googleapis.com", key_source: "workspace" },
  ];
  const router = withSession([
    { match: /\/workspaces\/ws\/land-gate\/advisor$/, body: () => ({ enabled: true, key_source: "workspace", provider: "typesafe", model: "jev-latest", host: "api.typesafe.ai", providers }) },
    advice(() => ({ ...green, provider: "google", model: "gemini-decision-1" })),
  ]);
  // The read of what is set up answers late.
  const fn: typeof router.fn = async (url, init) => {
    if (init?.method === "POST") asked.push(String(init.body));
    else if (/land-gate\/advisor$/.test(String(url))) await held;
    return router.fn(url, init);
  };
  await renderSignedIn(<Advice tid={TASK} />, fn);
  await new Promise((r) => setTimeout(r, 150));
  expect(asked, "nothing is sent until the choice is known to be askable").toEqual([]);
  release();
  const panel = await screen.findByRole("region", { name: "Advice" });
  await within(panel).findByText("Nothing flagged");
  expect(asked).toEqual([JSON.stringify({ provider: "google" })]);
  window.localStorage.removeItem("maidan_advice_provider");
});
