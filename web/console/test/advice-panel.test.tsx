import { renderSignedIn, withSession, type Route } from "./console-harness.tsx";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
  within(panel).getByText("None of the ten judgments raised a concern.");
  within(panel).getByText("Jev 1.14.0 by TypeSafe");
  within(panel).getByText(/Advice only: it approves and blocks nothing\. You decide\./);
  expect(panel.querySelector("details")?.open).toBe(false);
  await user.click(within(panel).getByText("The ten judgments"));
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
    ["Concern: Reports no passing tests, 98% sure", "concern"],
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
  await screen.findByRole("link", { name: "Add your TypeSafe key under Tools, Workspace." });
  cleanup();
  const down = withSession([advice(() => ({ detail: "land-gate advisor unavailable" }), 502)]);
  await renderSignedIn(<Advice tid={TASK} />, down.fn);
  await screen.findByText(/The task is unchanged/);
  screen.getByRole("button", { name: "Ask again" });
});
