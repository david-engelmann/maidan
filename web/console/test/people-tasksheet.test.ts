import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import { TRANSITION_WORDS, type Thread } from "../src/core/board.ts";
import {
  authorLabel,
  freshRoster,
  channelTag,
  holding,
  lastActiveByMember,
  lastActiveFact,
  mergeChannelThreads,
  workOf,
  workStamp,
} from "../src/core/people.ts";
import {
  DEP_FETCH_LIMIT,
  dependencyCue,
  dependencyIds,
  dependencyLabel,
  historyNewestFirst,
  loadTaskDependencies,
  mapBounded,
  transitionAt,
  transitionPhrase,
  type HistoryTransition,
} from "../src/core/taskSheet.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };

const T = (o: Partial<Thread> & { id: string }) => ({ state: "open", updated_at: "2026-10-30T10:00:00Z", ...o }) as Thread;

test("transition history is newest first, in the sheet's own words", () => {
  const rows: HistoryTransition[] = [
    { actor_id: "a", from_state: "open", to_state: "in_review", occurred_at: "2026-10-01T00:00:00Z" },
    { actor_id: "b", to_state: "closed", at: "2026-10-03T00:00:00Z" },
    { actor_id: "c", to_state: "archived", created_at: "2026-10-02T00:00:00Z" },
    { actor_id: "d", to_state: "open" },
  ];
  assert.equal(transitionAt(rows[0]), "2026-10-01T00:00:00Z");
  assert.equal(transitionAt(rows[1]), "2026-10-03T00:00:00Z");
  assert.equal(transitionAt({}), null);
  const ordered = historyNewestFirst(rows);
  assert.deepEqual(ordered.map((r) => r.actor_id), ["b", "c", "a", "d"]);
  assert.equal(transitionPhrase(rows[0]), TRANSITION_WORDS.start_review);
  assert.equal(transitionPhrase(rows[1]), TRANSITION_WORDS.close);
  assert.equal(transitionPhrase(rows[2]), TRANSITION_WORDS.archive);
  assert.equal(transitionPhrase({ to_state: "open" }), "Open");
  assert.equal(transitionPhrase({ to_state: "needs_input" }), "needs input");
  assert.equal(transitionPhrase({}), "Updated");
});

test("dependency ids keep wire order and drop blanks and duplicates", () => {
  assert.deepEqual(dependencyIds(null), []);
  assert.deepEqual(dependencyIds({ ready: true }), []);
  assert.deepEqual(
    dependencyIds({
      dependencies: [
        { depends_on_thread_id: "one" },
        { depends_on_thread_id: "" },
        { depends_on_thread_id: 4 },
        { depends_on_thread_id: "one" },
        { depends_on_thread_id: "two" },
      ],
      ready: false,
    }),
    ["one", "two"],
  );
});

test("a dependency is titled and cued without falling back to its id", () => {
  assert.equal(dependencyLabel({ title: " Ship it " }), "Ship it");
  assert.equal(dependencyLabel({ title: "  " }), "Untitled task");
  assert.equal(dependencyLabel({ title: null }), "Untitled task");
  assert.equal(dependencyLabel(null), "Unavailable task");
  assert.equal(dependencyLabel(null).includes("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"), false);
  assert.equal(dependencyCue("closed"), "(done)");
  assert.equal(dependencyCue("archived"), "(done)");
  assert.equal(dependencyCue("in_review"), "(in review)");
  assert.equal(dependencyCue("open"), "(open)");
  assert.equal(dependencyCue(undefined), "");
  assert.equal(dependencyCue("needs_input"), "(needs input)");
});

test("dependency reads use the session path, bound concurrency, and survive a missing task", async () => {
  const calls: string[] = [];
  let inFlight = 0;
  let peak = 0;
  const threads: Record<string, unknown> = {
    one: { id: "one", title: "First", state: "open" },
    two: { id: "two", title: "Second", state: "in_review" },
  };
  const fetchFn = async (url: string) => {
    calls.push(url);
    inFlight += 1;
    peak = Math.max(peak, inFlight);
    await new Promise((r) => setTimeout(r, 5));
    inFlight -= 1;
    if (url.endsWith("/dependencies")) {
      return new Response(JSON.stringify({
        dependencies: [{ depends_on_thread_id: "one" }, { depends_on_thread_id: "two" }, { depends_on_thread_id: "gone" }],
        ready: false,
      }));
    }
    const id = url.split("/").pop() ?? "";
    if (!threads[id]) return new Response("missing", { status: 404 });
    return new Response(JSON.stringify(threads[id]));
  };
  const links = await loadTaskDependencies(fetchFn, session, "task");
  assert.deepEqual(links, [
    { id: "one", title: "First", cue: "(open)", href: "#/threads/t/one" },
    { id: "two", title: "Second", cue: "(in review)", href: "#/threads/t/two" },
    { id: "gone", title: "Unavailable task", cue: "", href: "#/threads/t/gone" },
  ]);
  assert.ok(calls[0].startsWith("http://h/ui/api/threads/task/dependencies"));
  assert.ok(calls.slice(1).every((u) => u.startsWith("http://h/ui/api/threads/")));
  assert.ok(peak <= DEP_FETCH_LIMIT);

  const tokenCalls: string[] = [];
  const tokenLinks = await loadTaskDependencies(async (url) => {
    tokenCalls.push(url);
    return new Response("no", { status: 500 });
  }, bearer, "task");
  assert.deepEqual(tokenLinks, []);
  assert.equal(tokenCalls[0], "http://h/threads/task/dependencies");
});

test("bounded mapping keeps order and a cap on in-flight work", async () => {
  let inFlight = 0;
  let peak = 0;
  const out = await mapBounded([1, 2, 3, 4, 5, 6], 2, async (n) => {
    inFlight += 1;
    peak = Math.max(peak, inFlight);
    await new Promise((r) => setTimeout(r, 5));
    inFlight -= 1;
    return n * 10;
  });
  assert.deepEqual(out, [10, 20, 30, 40, 50, 60]);
  assert.equal(peak, 2);
  assert.deepEqual(await mapBounded([], 4, async (n: number) => n), []);
});

test("holding and work span every channel, with the channel named when there are several", () => {
  const build = [
    T({ id: "b1", assignee_id: "ada", title: "Hold", updated_at: "2026-10-30T08:00:00Z" }),
    T({ id: "b2", assignee_id: "ada", state: "closed", title: "Shipped", updated_at: "2026-10-29T08:00:00Z" }),
  ];
  const desk = [
    T({ id: "d1", assignee_id: "ada", title: "Running", work_started_at: "2026-10-30T09:00:00Z", updated_at: "2026-10-30T09:00:00Z" }),
    T({ id: "b1", assignee_id: "ada", title: "Duplicate" }),
  ];
  const threads = mergeChannelThreads([
    { channel: { id: "ch-build", name: "build" }, threads: build },
    { channel: { id: "ch-desk", name: "desk" }, threads: desk },
  ]);
  assert.deepEqual(threads.map((t) => [t.id, t.channelName]), [
    ["b1", "build"],
    ["b2", "build"],
    ["d1", "desk"],
  ]);
  assert.equal(holding(threads).get("ada")?.id, "d1");
  const work = workOf("ada", threads);
  assert.deepEqual(work.now.map((t) => t.id), ["d1", "b1"]);
  assert.deepEqual(work.recent.map((t) => t.id), ["b2"]);
  assert.equal(workStamp(work.now[0]), "2026-10-30T09:00:00Z");
  assert.equal(channelTag("build", 2), " · #build");
  assert.equal(channelTag("build", 1), "");
  assert.equal(channelTag("  ", 3), "");
  const many = workOf("ada", Array.from({ length: 8 }, (_, i) => T({ id: `c${i}`, assignee_id: "ada", state: "closed", updated_at: `2026-10-${String(i + 1).padStart(2, "0")}T00:00:00Z` })));
  assert.equal(many.recent.length, 5);
  assert.equal(many.recent[0].id, "c7");
});

test("last active follows the holder and the person who filed the task", () => {
  const at = lastActiveByMember([
    T({ id: "1", assignee_id: "ada", updated_at: "2026-10-30T10:00:00Z", created_at: "2026-10-01T00:00:00Z", created_by: "bea" } as Thread),
    T({ id: "2", assignee_id: "ada", updated_at: "2026-10-29T10:00:00Z" }),
    { ...T({ id: "3", state: "closed" }), author_id: "cy", created_at: "2026-11-01T00:00:00Z" },
    T({ id: "4", assignee_id: "ada", updated_at: "not-a-time" }),
  ]);
  assert.equal(at.get("ada"), "2026-10-30T10:00:00Z");
  assert.equal(at.get("bea"), "2026-10-01T00:00:00Z");
  assert.equal(at.get("cy"), "2026-11-01T00:00:00Z");
  assert.deepEqual(lastActiveFact("online", at.get("ada")), { text: "Online now", iso: null });
  assert.deepEqual(lastActiveFact("away", null), { text: "Away", iso: null });
  assert.deepEqual(lastActiveFact("offline", at.get("ada")), { text: null, iso: "2026-10-30T10:00:00Z" });
  assert.deepEqual(lastActiveFact("offline", null), { text: "No activity yet", iso: null });
});

test("a review with no workspace does not wait on a roster that will never arrive", () => {
  const none = freshRoster(null);
  assert.equal(none.workspaceId, null);
  assert.equal(none.ready, true);
  assert.equal(authorLabel(none.ready, undefined), "Someone");
  const ws = freshRoster("ws");
  assert.equal(ws.ready, false);
  assert.equal(authorLabel(ws.ready, undefined), "…");
});

test("a review author waits for the roster instead of flashing Someone", () => {
  assert.equal(authorLabel(false, undefined), "…");
  assert.equal(authorLabel(false, "Ada"), "Ada");
  assert.equal(authorLabel(true, undefined), "Someone");
  assert.equal(authorLabel(true, ""), "Someone");
  assert.equal(authorLabel(true, undefined, "Unknown"), "Unknown");
});
