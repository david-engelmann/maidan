import { FetchRouter, renderSignedIn, withSession } from "./console-harness.tsx";
import { cleanup, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { Activity } from "../src/activity/Activity.tsx";
import { People } from "../src/people/People.tsx";
import { phaseOf } from "../src/shell/DataState.tsx";
import { OFFLINE } from "../src/core/api.ts";
import { PeopleApi, asksForInvites, missingChannels } from "../src/core/people.ts";

afterEach(() => cleanup());

test("rows on screen outrank a failure, a failure outranks loading, and loading outranks empty", () => {
  expect(phaseOf({ settled: false, error: null, count: 0 })).toBe("loading");
  expect(phaseOf({ settled: true, error: null, count: 0 })).toBe("empty");
  expect(phaseOf({ settled: true, error: "down", count: 0 })).toBe("error");
  expect(phaseOf({ settled: false, error: "down", count: 0 })).toBe("error");
  expect(phaseOf({ settled: true, error: "down", count: 2 })).toBe("ready");
  expect(phaseOf({ settled: false, error: null, count: 2 })).toBe("ready");
});

test("activity says it is loading while its first read is in flight, not that nothing happened", async () => {
  let answer: (rows: unknown[]) => void = () => {};
  const pending = new Promise<unknown[]>((r) => (answer = r));
  const router = withSession([
    { match: /\/workspaces\/ws\/events/, body: () => pending },
    { match: /\/workspaces\/ws\/channels$/, body: () => [] },
    { match: /\/workspaces\/ws\/members$/, body: () => [] },
  ]);
  await renderSignedIn(<Activity />, router.fn);
  expect(await screen.findByRole("status", { name: "Loading recent events" })).toBeTruthy();
  expect(screen.queryByText("Nothing has happened yet")).toBeNull();
  answer([]);
  expect(await screen.findByText("Nothing has happened yet")).toBeTruthy();
  expect(screen.queryByRole("status", { name: "Loading recent events" })).toBeNull();
});

test("an unreachable server reads as the shared offline sentence on People, not the browser's words", async () => {
  const api = new PeopleApi(async () => { throw new TypeError("Failed to fetch"); }, () => ({ base: "https://m.example", pastedToken: "", tokenSession: false, sessionMemberId: "m1" }), () => "session");
  await expect(api.invites("ws")).rejects.toThrow(OFFLINE);
});

test("a token without workspace:admin is never asked for invites; a sign-in or an admin token is", () => {
  expect(asksForInvites("bearer", { capabilities: ["workspace:read"] })).toBe(false);
  expect(asksForInvites("delegated", { capabilities: ["workspace:write"] })).toBe(false);
  expect(asksForInvites("bearer", null)).toBe(false);
  expect(asksForInvites("bearer", { capabilities: ["workspace:admin"] })).toBe(true);
  expect(asksForInvites("session", { capabilities: ["workspace:read"] })).toBe(true);
  // A browser session traded for a token keeps that token's grant.
  expect(asksForInvites("session", { capabilities: ["workspace:read"] }, true)).toBe(false);
  expect(asksForInvites("session", { capabilities: ["workspace:admin"] }, true)).toBe(true);
});

test("a channel that failed to load is named, not silently dropped", () => {
  expect(missingChannels([])).toBeNull();
  expect(missingChannels(["ops"])).toBe("Could not load the tasks in #ops, so this list may be missing some.");
  expect(missingChannels(["a", "b", "c"])).toBe("Could not load the tasks in #a, #b and #c, so this list may be missing some.");
});

const roster = [{ id: "me", display_name: "Ada", handle: "ada", kind: "human" }];

test("a failed invites read offers Try again, never the invite form", async () => {
  const router = withSession([
    { match: /\/workspaces\/ws\/members$/, body: () => roster },
    { match: /\/workspaces\/ws\/channels$/, body: () => [] },
    { match: /\/invites$/, status: 500, body: () => ({ error: "boom" }) },
  ]);
  await renderSignedIn(<People />, router.fn);
  await waitFor(() => expect(router.calls.some((c) => /\/invites$/.test(c.url))).toBe(true));
  await waitFor(() => expect(screen.getAllByRole("button", { name: "Try again" }).length).toBeGreaterThan(0));
  expect(screen.queryByRole("form", { name: "Invite a person" })).toBeNull();
});

test("a token without workspace:admin opens People without a single invites request", async () => {
  const router = new FetchRouter([
    { match: /\/auth\/session$/, body: () => ({ member_id: "me", workspace_id: "ws", display_name: "Ada" }) },
    { match: /\/me$/, body: () => ({ member_id: "me", workspace_id: "ws", is_bearer: true, capabilities: ["workspace:read"], known_capabilities: [] }) },
    { match: /\/workspaces\/ws\/members$/, body: () => roster },
    { match: /\/workspaces\/ws\/channels$/, body: () => [] },
    { match: /\/invites$/, body: () => [] },
  ]);
  await renderSignedIn(<People />, router.fn);
  await waitFor(() => expect(router.calls.some((c) => /\/members$/.test(c.url))).toBe(true));
  await new Promise((r) => setTimeout(r, 50));
  expect(router.calls.filter((c) => /\/invites$/.test(c.url))).toEqual([]);
  expect(screen.queryByRole("form", { name: "Invite a person" })).toBeNull();
});
