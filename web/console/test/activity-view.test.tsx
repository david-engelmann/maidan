import { renderSignedIn, withSession } from "./console-harness.tsx";
import { cleanup, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { Activity } from "../src/activity/Activity.tsx";

afterEach(() => cleanup());

const events = [
  {
    id: 4,
    kind: "thread_created",
    thread_id: "t1",
    channel_id: "c1",
    actor_id: "m1",
    occurred_at: "2026-11-01T00:00:00Z",
    payload: { thread: { id: "t1", title: "Ship the login" } },
  },
  {
    id: 5,
    kind: "thread_state_changed",
    thread_id: "t1",
    channel_id: "c1",
    actor_id: "m1",
    occurred_at: "2026-11-01T00:01:00Z",
    payload: { to_state: "in_review", thread: { id: "t1", title: "Ship the login" } },
  },
  {
    id: 6,
    kind: "message_posted",
    thread_id: "t-new",
    channel_id: "c1",
    actor_id: "m2",
    occurred_at: "2026-11-01T00:02:00Z",
    payload: {},
  },
  {
    id: 3,
    kind: "weird_custom_event",
    thread_id: "deadbeef-0000-7000-8000-000000000001",
    channel_id: "nochname",
    actor_id: "01945a8b-0000-7000-8000-000000000099",
    occurred_at: "2026-11-01T00:03:00Z",
    payload: {},
  },
];

test("activity names the task, the channel and who did it", async () => {
  const router = withSession([
    { match: /\/workspaces\/ws\/events/, body: () => events },
    { match: /\/workspaces\/ws\/channels$/, body: () => [{ id: "c1", name: "general" }] },
    {
      match: /\/workspaces\/ws\/members$/,
      body: () => [
        { id: "m1", display_name: "Ada", handle: "ada" },
        { id: "m2", handle: "codex" },
      ],
    },
    { match: /\/channels\/c1\/threads/, body: () => [{ id: "t1", title: "Ship the login", state: "open" }] },
    { match: /\/threads\/t-new$/, body: () => ({ id: "t-new", title: "A new DM", state: "open" }) },
    { match: /\/threads\/deadbeef-0000-7000-8000-000000000001$/, status: 404, body: () => ({ error: "nope" }) },
  ]);
  await renderSignedIn(<Activity />, router.fn);
  await waitFor(() => expect(screen.getAllByRole("link", { name: "Ship the login" })).toHaveLength(2));
  expect(screen.getByText("Task created")).toBeTruthy();
  expect(screen.getByText("Moved to in review")).toBeTruthy();
  await waitFor(() => {
    expect(screen.getAllByText("#general").length).toBeGreaterThan(0);
    expect(screen.getAllByText("by Ada").length).toBeGreaterThan(0);
    expect(screen.getByText("by @codex")).toBeTruthy();
  });
  expect(screen.getByText("Weird custom event")).toBeTruthy();
  await waitFor(() => expect(screen.getByRole("link", { name: "A new DM" })).toBeTruthy());
  expect(screen.getByRole("link", { name: "thread deadbeef" })).toBeTruthy();
  await waitFor(() => expect(router.calls.filter((c) => /\/threads\/t-new$/.test(c.url))).toHaveLength(1));
  await waitFor(() => expect(router.calls.filter((c) => /\/threads\/deadbeef-/.test(c.url))).toHaveLength(1));
  await new Promise((resolve) => setTimeout(resolve, 40));
  expect(router.calls.filter((c) => /\/threads\/t-new$/.test(c.url))).toHaveLength(1);
  expect(router.calls.filter((c) => /\/threads\/deadbeef-/.test(c.url))).toHaveLength(1);
  expect(screen.getAllByRole("link", { name: "Ship the login" })[0].getAttribute("href")).toBe("#/threads/t/t1");
  expect(screen.getByRole("link", { name: "thread deadbeef" }).getAttribute("href")).toBe("#/threads/t/deadbeef-0000-7000-8000-000000000001");
  expect(screen.getByText("channel nochname")).toBeTruthy();
  expect(screen.getByText("by 01945a8b")).toBeTruthy();
});
