import { FetchRouter, renderSignedIn } from "./console-harness.tsx";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import { EndTaskForm, EndTaskRow } from "../src/board/EndTask.tsx";
import { EndTaskApi } from "../src/core/endTask.ts";
import type { Credentials } from "../src/core/api.ts";
import type { Thread } from "../src/core/board.ts";

afterEach(() => cleanup());

const TASK = "0190a1b2-0000-7000-8000-0000000000aa";
const TWIN = "0190a1b2-0000-7000-8000-0000000000bb";
const th = (over: Partial<Thread> = {}): Thread => ({ id: TASK, title: "Fix login", state: "open", created_by: "me", ...over }) as Thread;

type Sent = { url: string; method: string; body: unknown };

function api(status: number, body: unknown) {
  const sent: Sent[] = [];
  const creds: Credentials = { base: "https://console.test", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
  const client = new EndTaskApi(
    async (url, init) => {
      sent.push({ url, method: init?.method ?? "GET", body: init?.body ? JSON.parse(String(init.body)) : null });
      return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    },
    () => creds,
    () => "session",
  );
  return { client, sent };
}

test("a cancel asks why, says what it does, then sends it", async () => {
  const user = userEvent.setup();
  const { client, sent } = api(200, { thread: { ...th(), state: "closed" }, replayed: false });
  const onDone = vi.fn();
  render(<EndTaskForm th={th()} kind="cancel" api={client} onDone={onDone} onBack={() => {}} />);

  const why = screen.getByRole("textbox", { name: /Why cancel it/ });
  await waitFor(() => expect(document.activeElement).toBe(why));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByRole("alert").textContent).toMatch(/Say why/);
  expect(sent).toHaveLength(0);

  await user.type(why, "Nobody needs it");
  await user.click(screen.getByRole("button", { name: "Continue" }));
  const heading = screen.getByRole("heading", { name: "Cancel “Fix login”?" });
  await waitFor(() => expect(document.activeElement).toBe(heading));
  expect(screen.getByRole("list").textContent).toMatch(/claim ends at once/);
  expect(sent).toHaveLength(0);

  await user.click(screen.getByRole("button", { name: "Cancel task" }));
  await waitFor(() => expect(onDone).toHaveBeenCalledWith("Cancelled. The task is closed.", expect.objectContaining({ state: "closed" })));
  expect(sent).toEqual([{ url: `https://console.test/ui/api/threads/${TASK}/cancel`, method: "POST", body: { reason: "Nobody needs it" } }]);
});

test("a duplicate close needs the other task, and a refusal reads calmly on the confirm step", async () => {
  const user = userEvent.setup();
  const { client, sent } = api(409, {
    detail: "separation of duties: the claimer cannot land its own work on an owned thread; the owner or another member must perform this transition",
  });
  const onDone = vi.fn();
  render(<EndTaskForm th={th()} kind="abandon" api={client} onDone={onDone} onBack={() => {}} />);

  await user.click(screen.getByRole("radio", { name: "Another task already covers it" }));
  await user.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByRole("alert").textContent).toMatch(/Paste a link/);

  await user.type(screen.getByRole("textbox", { name: /covers it/ }), `https://console.test/ui/#/threads/t/${TWIN}`);
  await user.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByRole("list").textContent).toMatch(/move to the task you name/);
  await user.click(screen.getByRole("button", { name: "Close task" }));

  await waitFor(() => expect(screen.getByRole("alert").textContent).toMatch(/someone else closes it/));
  expect(onDone).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Close task" })).not.toHaveProperty("disabled", true);
  expect(sent[0].body).toEqual({ reason: "duplicate", duplicate_of: TWIN });
});

const MEMBER = ["workspace:read", "workspace:write", "message:post", "thread:transition"];

function session(capabilities: string[]): FetchRouter {
  return new FetchRouter([
    { match: /\/auth\/session$/, body: () => ({ member_id: "me", workspace_id: "ws", display_name: "Ada" }) },
    { match: /\/me$/, body: () => ({ member_id: "me", workspace_id: "ws", is_bearer: false, capabilities, known_capabilities: MEMBER }) },
  ]);
}

test("the thread view offers what the session may do, and Keep the task gives focus back", async () => {
  const user = userEvent.setup();
  await renderSignedIn(<EndTaskRow th={th()} onChanged={() => {}} />, session(MEMBER).fn);

  const abandon = await screen.findByRole("button", { name: "Close as abandoned…" });
  expect(screen.getByRole("button", { name: "Cancel task…" })).toBeTruthy();
  await user.click(abandon);
  await user.click(screen.getByRole("button", { name: "Keep the task" }));
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: "Close as abandoned…" })));
});

test("the thread view offers nothing to a session that may not end the task", async () => {
  await renderSignedIn(<EndTaskRow th={th({ created_by: "someone" })} onChanged={() => {}} />, session(["workspace:read", "workspace:write"]).fn);
  await waitFor(() => expect(screen.queryByRole("group", { name: "End this task" })).toBeNull());
  expect(screen.queryByRole("button", { name: /Cancel task/ })).toBeNull();
});
