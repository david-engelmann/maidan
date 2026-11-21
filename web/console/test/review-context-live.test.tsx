import { renderSignedIn, withSession, type Route } from "./console-harness.tsx";
import { act, cleanup, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { liveBus } from "../src/core/bus.ts";
import { ReviewContext, changesContext } from "../src/needs/ReviewContext.tsx";

afterEach(() => cleanup());

const TASK = "0190a1b2-0000-7000-8000-0000000000aa";
const OTHER = "0190a1b2-0000-7000-8000-0000000000bb";
const event = (kind: string, thread_id: string) => ({ type: "event" as const, frame: { log_id: 1, kind, thread_id } });

test("a result or a message on this task is a change; another task's, or a claim, is not", () => {
  expect(changesContext(event("thread_result_set", TASK), TASK)).toBe(true);
  expect(changesContext(event("message_posted", TASK), TASK)).toBe(true);
  expect(changesContext({ type: "resync" }, TASK)).toBe(true);
  expect(changesContext(event("thread_result_set", OTHER), TASK)).toBe(false);
  expect(changesContext(event("thread_assignment_changed", TASK), TASK)).toBe(false);
});

test("an open task's result and messages appear when an agent sets them, without reopening it", async () => {
  // The task is open on screen before the agent has done anything.
  let result: unknown = null;
  let messages: unknown[] = [];
  const routes: Route[] = [
    { match: new RegExp(`/threads/${TASK}/result$`), body: () => result, get status() { return result ? 200 : 404; } },
    { match: new RegExp(`/threads/${TASK}/messages`), body: () => messages },
    { match: /\/workspaces\/ws\/members/, body: () => [{ id: "a1", name: "Claude Code", kind: "agent" }] },
  ];
  const router = withSession(routes);
  await renderSignedIn(<ReviewContext tid={TASK} shas={[]} />, router.fn);
  await screen.findByText("No result recorded yet.");
  await screen.findByText("No messages yet.");

  // The agent posts and sets its result; the server says so on the live stream.
  result = { thread_id: TASK, result: { status: "changed", summary: "Adds a token-bucket limiter." } };
  messages = [{ id: "m1", thread_id: TASK, author_id: "a1", body: "cargo test: 212 passed.", posted_at: "2026-11-21T03:00:00Z" }];
  // Another task's events change nothing here.
  act(() => liveBus.emit(event("thread_result_set", OTHER)));
  await new Promise((r) => setTimeout(r, 400));
  expect(screen.queryByText(/token-bucket limiter/)).toBeNull();

  act(() => {
    liveBus.emit(event("message_posted", TASK));
    liveBus.emit(event("thread_result_set", TASK));
  });
  await waitFor(() => expect(screen.getByText(/token-bucket limiter/)).toBeTruthy());
  await screen.findByText(/212 passed/);
  expect(screen.queryByText("No result recorded yet.")).toBeNull();
});

test("a task whose events arrive faster than a read answers still shows what the reads find", async () => {
  let result: unknown = null;
  const routes: Route[] = [
    { match: new RegExp(`/threads/${TASK}/result$`), body: () => result, get status() { return result ? 200 : 404; } },
    { match: new RegExp(`/threads/${TASK}/messages`), body: () => [] },
    { match: /\/workspaces\/ws\/members/, body: () => [] },
  ];
  const router = withSession(routes);
  // Every read of the result takes 600 ms to answer.
  const slow: typeof router.fn = async (url, init) => {
    const res = await router.fn(url, init);
    if (/\/result$/.test(String(url))) await new Promise((r) => setTimeout(r, 600));
    return res;
  };
  await renderSignedIn(<ReviewContext tid={TASK} shas={[]} />, slow);
  await screen.findByText("No result recorded yet.");

  result = { thread_id: TASK, result: { summary: "Adds a token-bucket limiter." } };
  // An event every 400 ms for two seconds: each starts a read before the last one answered.
  const busy = setInterval(() => act(() => liveBus.emit(event("message_posted", TASK))), 400);
  act(() => liveBus.emit(event("thread_result_set", TASK)));
  try {
    // The first of those reads answers about 850 ms in, while events still arrive.
    await waitFor(() => expect(screen.getByText(/token-bucket limiter/)).toBeTruthy(), { timeout: 1500 });
  } finally {
    clearInterval(busy);
  }
});
