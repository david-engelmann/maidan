import { test } from "node:test";
import assert from "node:assert/strict";
import { absorbPayloadNames, activityChannels, actorText, actorWords, channelText, describeActivity, describeKind, directoryAfterLoad, EMPTY_DIRECTORY, filterActivity, groupOf, mergeActivity, threadText, toActivity, type ActivityDirectory, type ActivityRow } from "../src/core/activity.ts";
import { ROUTES, parseRoute } from "../src/shell/router.ts";

const blank = { title: null, channelName: null, actorName: null, actorHandle: null, toState: null };

test("REST rows and live frames map to one shape", () => {
  assert.deepEqual(toActivity({ id: 5, kind: "thread_created", thread_id: "t1", channel_id: "c1", payload: { actor_id: "m1" }, occurred_at: "2026-11-01T00:00:00Z" }), {
    id: 5, kind: "thread_created", threadId: "t1", channelId: "c1", actorId: "m1", at: "2026-11-01T00:00:00Z", ...blank,
  });
  assert.deepEqual(toActivity({ log_id: 6, kind: "vote_cast", thread_id: "t1", member_id: "m2" }), {
    id: 6, kind: "vote_cast", threadId: "t1", channelId: null, actorId: "m2", at: null, ...blank,
  });
  assert.equal(toActivity({ kind: "x" }), null);
});

test("a payload contributes the title, channel, actor and destination state", () => {
  const row = toActivity({
    id: 7,
    kind: "thread_state_changed",
    payload: {
      to_state: "in_review",
      thread: { id: "t1", title: "Ship the login" },
      channel: { id: "c1", name: "general" },
      member: { id: "m1", display_name: "Ada", handle: "ada" },
    },
  });
  assert.equal(row?.threadId, "t1");
  assert.equal(row?.channelId, "c1");
  assert.equal(row?.actorId, "m1");
  assert.equal(row?.title, "Ship the login");
  assert.equal(row?.channelName, "general");
  assert.equal(row?.actorName, "Ada");
  assert.equal(row?.actorHandle, "ada");
  assert.equal(row?.toState, "in_review");
  assert.equal(describeActivity(row!), "Moved to in review");
});

test("kinds read as sentences, and an unknown kind is capitalized", () => {
  assert.equal(describeKind("thread_created"), "Task created");
  assert.equal(describeKind("message_posted"), "Message posted");
  assert.equal(describeKind("approval_requested"), "Approval requested");
  assert.equal(describeKind("approval_resolved"), "Approval answered");
  assert.equal(describeKind("review_submitted"), "Review submitted");
  assert.equal(describeKind("vote_cast"), "Vote cast");
  assert.equal(describeKind("totally_unknown_kind"), "Totally unknown kind");
  assert.equal(describeActivity({ kind: "thread_state_changed", toState: null }), "State changed");
  assert.equal(describeActivity({ kind: "thread_state_changed", toState: "closed" }), "Moved to closed");
});

test("names fall back to a short id, and a handle is shown with @", () => {
  const id = "3f8a91c2-0000-7000-8000-000000000001";
  const row = { threadId: id, channelId: id, actorId: id, title: null, channelName: null, actorName: null, actorHandle: null };
  assert.equal(threadText(row, EMPTY_DIRECTORY), "thread 3f8a91c2");
  assert.equal(channelText(row, EMPTY_DIRECTORY), "channel 3f8a91c2");
  assert.equal(actorText(row, EMPTY_DIRECTORY), "3f8a91c2");
  assert.equal(actorWords({ id, name: "Ada", handle: "ada" }), "Ada");
  assert.equal(actorWords({ id, name: "codex", handle: "codex" }), "@codex");
  assert.equal(actorWords({ id, name: id.slice(0, 8) }), null);
  const named = absorbPayloadNames(EMPTY_DIRECTORY, [{
    id: 1, kind: "message_posted", threadId: "t1", channelId: "c1", actorId: "m1", at: null,
    title: "Ship the login", channelName: "#general", actorName: null, actorHandle: "codex", toState: null,
  }]);
  assert.equal(threadText({ threadId: "t1", title: null }, named), "Ship the login");
  assert.equal(channelText({ channelId: "c1", channelName: null }, named), "#general");
  assert.equal(actorText({ actorId: "m1", actorName: null, actorHandle: null }, named), "@codex");
});

test("merge dedupes by id, newest first, capped", () => {
  const r = (id: number): ActivityRow => ({ id, kind: "k", threadId: null, channelId: null, actorId: null, at: null, title: null, channelName: null, actorName: null, actorHandle: null, toState: null });
  assert.deepEqual(mergeActivity([r(1), r(3)], [r(3), r(2), r(4)]).map((x) => x.id), [4, 3, 2, 1]);
  assert.deepEqual(mergeActivity([r(1), r(2), r(3)], [], 2).map((x) => x.id), [3, 2]);
});

test("groups filter by kind family", () => {
  assert.equal(groupOf("vote_retracted"), "votes");
  assert.equal(groupOf("reaction_added"), "messages");
  assert.equal(groupOf("thread_state_changed"), "tasks");
  const rows = [toActivity({ id: 1, kind: "vote_cast" })!, toActivity({ id: 2, kind: "message_posted" })!];
  assert.deepEqual(filterActivity(rows, "votes").map((x) => x.id), [1]);
  assert.equal(filterActivity(rows, "all").length, 2);
});

const row = (id: number, kind: string, extra: Partial<ActivityRow> = {}): ActivityRow => ({
  id, kind, threadId: null, channelId: null, actorId: null, at: null, title: null, channelName: null, actorName: null, actorHandle: null, toState: null, ...extra,
});

test("search and channel narrow the list, and a channel menu names every channel in view", () => {
  const dir: ActivityDirectory = {
    threads: new Map([["t1", "Deploy v9"]]),
    channels: new Map([["c1", "general"]]),
    actors: new Map([["m1", "Ada"]]),
  };
  const rows = [
    row(1, "thread_created", { threadId: "t1", channelId: "c1", actorId: "m1", title: "Deploy v9" }),
    row(2, "message_posted", { threadId: "t2", channelId: "c2", actorId: "m2", channelName: "ops", title: "Notes" }),
    row(3, "vote_cast", { threadId: "t1", channelId: "c1", actorId: "m1" }),
  ];
  assert.deepEqual(filterActivity(rows, "tasks", "", "", dir).map((r) => r.id), [1]);
  assert.deepEqual(filterActivity(rows, "all", "deploy", "", dir).map((r) => r.id), [1, 3]);
  assert.deepEqual(filterActivity(rows, "all", "ada", "c1", dir).map((r) => r.id), [1, 3]);
  assert.deepEqual(filterActivity(rows, "messages", "ada", "", dir).map((r) => r.id), []);
  assert.deepEqual(filterActivity(rows, "all", "#2", "", dir).map((r) => r.id), [2]);
  assert.deepEqual(filterActivity(rows, "all", "message_posted", "", dir).map((r) => r.id), [2]);
  assert.deepEqual(activityChannels(rows, dir), [
    { id: "c1", label: "#general" },
    { id: "c2", label: "#ops" },
  ]);
});

test("a new workspace starts from an empty directory, so the previous room's titles do not carry over", () => {
  const previous: ActivityDirectory = {
    threads: new Map([["t-old", "Deploy v9"]]),
    channels: new Map([["c-old", "general"]]),
    actors: new Map([["m-old", "Ada"]]),
  };
  const fresh = directoryAfterLoad(null, EMPTY_DIRECTORY, []);
  assert.equal(fresh.threads.has("t-old"), false);
  assert.equal(fresh.channels.has("c-old"), false);
  assert.equal(fresh.actors.has("m-old"), false);
  const sameRoom = directoryAfterLoad(previous, EMPTY_DIRECTORY, []);
  assert.equal(sameRoom.threads.get("t-old"), "Deploy v9");
  const renamed = directoryAfterLoad(previous, { ...EMPTY_DIRECTORY, threads: new Map([["t-old", "Deploy v10"]]) }, []);
  assert.equal(renamed.threads.get("t-old"), "Deploy v10");
});

test("activity is a route", () => {
  assert.ok(ROUTES.some((r) => r.id === "activity"));
  assert.equal(parseRoute("#/activity"), "activity");
});

test("a freeze row names who froze, not the frozen member", () => {
  assert.equal(toActivity({ id: 9, kind: "member_frozen", member_id: "frozen", payload: { frozen_by: "admin" } })?.actorId, "admin");
  assert.equal(toActivity({ id: 10, kind: "member_unfrozen", member_id: "frozen", unfrozen_by: "admin" })?.actorId, "admin");
});
