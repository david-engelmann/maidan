import { renderSignedIn, withSession } from "./console-harness.tsx";
import { act, cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, expect, test, vi } from "vitest";
import { TaskSheet } from "../src/board/TaskSheet.tsx";
import type { Credentials } from "../src/core/api.ts";
import { BoardApi, type Thread } from "../src/core/board.ts";

// The sheet's spring has nothing to do with focus, and happy-dom rejects a
// cancelled animation at unmount.
vi.mock("motion/react", async () => {
  const React = await import("react");
  const drop = new Set(["initial", "animate", "exit", "transition"]);
  const div = (props: Record<string, unknown>) => {
    const next: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(props)) if (!drop.has(key)) next[key] = value;
    return React.createElement("div", next);
  };
  return {
    AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
    motion: { div },
    useReducedMotion: () => true,
  };
});

afterEach(() => cleanup());

const TASK = "0190a1b2-0000-7000-8000-0000000000aa";
const th = { id: TASK, title: "Fix login", state: "in_review", created_by: "me", assignee_id: "agent" } as Thread;

// The board re-renders on a clock tick, a refresh and every live event, and
// hands the sheet a new onClose each time. `rerenderBoard` is that render.
let rerenderBoard = () => {};
function Board({ api }: { api: BoardApi }) {
  const [now, setNow] = useState(1);
  rerenderBoard = () => setNow((n) => n + 1);
  return <TaskSheet thread={th} members={new Map()} api={api} now={now} onClose={() => {}} onChanged={() => {}} />;
}

test("the board re-rendering leaves an open menu open and focus where it was", async () => {
  const user = userEvent.setup();
  const router = withSession([]);
  const creds: Credentials = { base: "https://console.test", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
  const api = new BoardApi(router.fn, () => creds, () => "session");
  await renderSignedIn(<Board api={api} />, router.fn);

  const sheet = await screen.findByRole("dialog");
  await waitFor(() => expect(document.activeElement?.getAttribute("aria-label")).toBe("Close the task sheet"));

  await user.click(screen.getByRole("button", { name: "More actions" }));
  const first = screen.getAllByRole("menuitem")[0];
  await waitFor(() => expect(document.activeElement).toBe(first));

  act(() => rerenderBoard());
  act(() => rerenderBoard());

  expect(screen.queryByRole("menu")).not.toBeNull();
  expect(document.activeElement).toBe(first);
  expect(sheet.contains(document.activeElement)).toBe(true);
});
