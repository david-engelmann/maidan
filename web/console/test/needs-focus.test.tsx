import { renderSignedIn, withSession, type Route } from "./console-harness.tsx";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { liveBus } from "../src/core/bus.ts";
import { NeedsYou } from "../src/needs/NeedsYou.tsx";

vi.mock("motion/react", async () => {
  const React = await import("react");
  const drop = new Set(["layout", "initial", "animate", "exit", "transition", "drag", "dragConstraints", "dragElastic", "dragSnapToOrigin", "onDragEnd"]);
  const host = (tag: "li" | "div") => (props: Record<string, unknown>) => {
    const next: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(props)) if (!drop.has(key)) next[key] = value;
    return React.createElement(tag, next);
  };
  return {
    AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
    motion: { li: host("li"), div: host("div") },
    useReducedMotion: () => true,
  };
});

afterEach(() => cleanup());

const blocked = (id: string, summary: string) => ({
  kind: "blocked",
  thread_id: id,
  summary,
  since: "2026-11-01T00:00:00Z",
  age_secs: 90,
});

function shell(ui: ReactNode, routes: Route[], withMain = true) {
  const router = withSession(routes);
  const node = withMain ? (
    <main id="main" tabIndex={-1}>
      {ui}
    </main>
  ) : (
    ui
  );
  return { router, node };
}

test("confirm and note keep focus, and Escape or Cancel gives it back", async () => {
  const user = userEvent.setup();
  let release: (value: { evidence_root: string; self_reported_only: boolean; manifest: { artifacts: string[] } }) => void = () => {};
  const pending = new Promise<{ evidence_root: string; self_reported_only: boolean; manifest: { artifacts: string[] } }>((resolve) => {
    release = resolve;
  });
  const { router, node } = shell(<NeedsYou />, [
    {
      match: /\/members\/me\/waiting$/,
      body: () => ({
        items: [
          blocked("t-block", "Held up"),
          { kind: "open_gate", thread_id: "t-gate", gate_id: "g1", summary: "Deploy?", since: "2026-11-01T00:00:00Z", age_secs: 30 },
          { kind: "review_request", thread_id: "t-review", summary: "Please review", since: "2026-11-01T00:00:00Z", age_secs: 10 },
        ],
      }),
    },
    { match: /\/approval-gates$/, body: () => [{ gate: { id: "g1", prompt: "Deploy?" }, request_state: "sig" }] },
    { match: /\/threads\/t-review\/review-packet$/, body: () => pending },
    { match: /\/threads\/t-review\/artifacts$/, body: () => [] },
    { match: /\/threads\/t-block\/block$/, method: "DELETE", body: () => ({}) },
  ]);
  await renderSignedIn(node, router.fn);

  const block = await waitFor(() => screen.getByRole("button", { name: /Held up/ }).closest("li"));
  if (!block) throw new Error("missing blocked row");
  const unblock = within(block).getByRole("button", { name: "Unblock" });
  unblock.focus();
  await user.click(unblock);
  const confirm = block.querySelector(".need-confirm .btn.go");
  expect(confirm).toBeTruthy();
  await waitFor(() => expect(document.activeElement).toBe(confirm));
  await user.click(within(block).getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(document.activeElement).toBe(within(block).getByRole("button", { name: "Unblock" })));

  await user.click(within(block).getByRole("button", { name: "Unblock" }));
  await waitFor(() => expect(document.activeElement).toBe(block.querySelector(".need-confirm .btn.go")));
  await user.keyboard("{Escape}");
  await waitFor(() => expect(document.activeElement).toBe(within(block).getByRole("button", { name: "Unblock" })));

  const gate = screen.getByRole("button", { name: /Deploy\?/ }).closest("li");
  if (!gate) throw new Error("missing gate row");
  await user.click(within(gate).getByRole("button", { name: /Decline/ }));
  const note = gate.querySelector("textarea");
  await waitFor(() => expect(document.activeElement).toBe(note));
  await user.keyboard("{Escape}");
  await waitFor(() => expect(document.activeElement).toBe(within(gate).getByRole("button", { name: /Decline/ })));

  const review = screen.getByRole("button", { name: /Please review/ }).closest("li");
  if (!review) throw new Error("missing review row");
  const send = within(review).getByRole("button", { name: /Send back/ });
  send.focus();
  expect(document.activeElement).toBe(send);
  release({ evidence_root: "root", self_reported_only: false, manifest: { artifacts: [] } });
  await waitFor(() => expect(review.querySelector(".need-evidence")).toBeTruthy());
  expect(document.activeElement).toBe(within(review).getByRole("button", { name: /Send back/ }));

  // The note replaces the action row, so the clicked button is gone. Escape
  // has to land on the new Send back, not the first button in the row.
  await user.click(within(review).getByRole("button", { name: /Send back/ }));
  await waitFor(() => expect(document.activeElement).toBe(review.querySelector("textarea")));
  await user.keyboard("{Escape}");
  await waitFor(() => expect(document.activeElement).toBe(within(review).getByRole("button", { name: /Send back/ })));
});

test("r focuses the note while the queue pick stays focused", async () => {
  const user = userEvent.setup();
  const { router, node } = shell(<NeedsYou />, [
    {
      match: /\/members\/me\/waiting$/,
      body: () => ({
        items: [{ kind: "review_request", thread_id: "t-review", summary: "Please review", since: "2026-11-01T00:00:00Z", age_secs: 10 }],
      }),
    },
    { match: /\/approval-gates$/, body: () => [] },
    { match: /\/threads\/t-review\/review-packet$/, body: () => ({ evidence_root: "root", self_reported_only: false, manifest: { artifacts: [] } }) },
    { match: /\/threads\/t-review\/artifacts$/, body: () => [] },
  ]);
  await renderSignedIn(node, router.fn);
  const pick = await waitFor(() => screen.getByRole("button", { name: /Please review/ }));
  pick.focus();
  expect(document.activeElement).toBe(pick);
  await user.keyboard("r");
  const note = document.querySelector("textarea");
  await waitFor(() => expect(document.activeElement).toBe(note));
  await user.keyboard("{Escape}");
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: /Send back/ })));
});

test("answering a row moves focus to the next one", async () => {
  const user = userEvent.setup();
  const { router, node } = shell(<NeedsYou />, [
    { match: /\/members\/me\/waiting$/, body: () => ({ items: [blocked("t1", "First hold"), blocked("t2", "Second hold")] }) },
    { match: /\/approval-gates$/, body: () => [] },
    { match: /\/threads\/t1\/block$/, method: "DELETE", body: () => ({}) },
  ]);
  await renderSignedIn(node, router.fn);
  const first = await waitFor(() => screen.getByRole("button", { name: /First hold/ }).closest("li"));
  if (!first) throw new Error("missing first row");
  await user.click(within(first).getByRole("button", { name: "Unblock" }));
  const go = first.querySelector(".need-confirm .btn.go");
  if (!(go instanceof HTMLElement)) throw new Error("missing confirm");
  await waitFor(() => expect(document.activeElement).toBe(go));
  await user.click(go);
  await waitFor(() => {
    const next = document.querySelector('[data-need-key="blocked:t2:"] .need-pick');
    expect(document.activeElement).toBe(next);
  });
});

test("clearing the last row moves focus to the page, and a refresh keeps it", async () => {
  const user = userEvent.setup();
  let items = [blocked("t1", "Only hold")];
  const { router, node } = shell(<NeedsYou />, [
    { match: /\/members\/me\/waiting$/, body: () => ({ items }) },
    { match: /\/approval-gates$/, body: () => [] },
  ]);
  await renderSignedIn(node, router.fn);
  const pick = await waitFor(() => screen.getByRole("button", { name: /^Only hold / }));
  pick.focus();
  expect(document.activeElement).toBe(pick);
  items = [{ ...blocked("t1", "Only hold"), summary: "Only hold, still" }];
  liveBus.emit({ type: "event", frame: { log_id: 9, kind: "thread_state_changed" } });
  await waitFor(() => expect(screen.getByRole("button", { name: /Only hold, still/ })).toBeTruthy());
  expect(document.activeElement).toBe(screen.getByRole("button", { name: /Only hold, still/ }).closest("li")?.querySelector(".need-pick"));

  items = [];
  liveBus.emit({ type: "event", frame: { log_id: 10, kind: "thread_state_changed" } });
  await waitFor(() => expect(document.querySelector(".needs-empty")).toBeTruthy());
  await waitFor(() => expect(document.activeElement).toBe(document.getElementById("main")));

  cleanup();
  items = [blocked("t9", "Last one")];
  const bare = shell(<NeedsYou />, [
    { match: /\/members\/me\/waiting$/, body: () => ({ items }) },
    { match: /\/approval-gates$/, body: () => [] },
    { match: /\/threads\/t9\/block$/, method: "DELETE", body: () => ({}) },
  ], false);
  await renderSignedIn(bare.node, bare.router.fn);
  const row = await waitFor(() => screen.getByRole("button", { name: /Last one/ }).closest("li"));
  if (!row) throw new Error("missing last row");
  await user.click(within(row).getByRole("button", { name: "Unblock" }));
  const go = row.querySelector(".need-confirm .btn.go");
  if (!(go instanceof HTMLElement)) throw new Error("missing confirm");
  await user.click(go);
  items = [];
  await waitFor(() => expect(document.querySelector(".needs-empty")).toBeTruthy(), { timeout: 3000 });
  await waitFor(() => expect(document.activeElement?.id).toBe("needs-title"));
}, 8000);
