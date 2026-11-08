import assert from "node:assert/strict";
import { test } from "node:test";
import type { SearchHit } from "../src/core/tools.ts";
import { channelControl } from "../src/core/boardQuery.ts";
import { routeCommands, type Command } from "../src/shell/commands.ts";
import {
  PALETTE_PLACEHOLDER_IN,
  PALETTE_PLACEHOLDER_OUT,
  PALETTE_SEARCH_MIN,
  clearPaletteIndex,
  loadPaletteIndex,
  messageCommands,
  openThreadHash,
  paletteEmpty,
  paletteItems,
  palettePlaceholder,
  peekPaletteIndex,
  storePaletteIndex,
  type PaletteChannel,
  type PalettePerson,
  type PaletteTask,
} from "../src/shell/paletteIndex.ts";

const channels: PaletteChannel[] = [
  { id: "c-ops", name: "ops" },
  { id: "c-gen", name: "general" },
];
const people: PalettePerson[] = [
  { id: "me", name: "Operator", handle: "operator", kind: "human" },
  { id: "ag", name: "Deployer", handle: "deployer", kind: "agent" },
];
const tasks: PaletteTask[] = [
  { id: "t1", title: "Deploy v9", channelName: "general" },
  { id: "t2", title: "Open: nobody holds this", channelName: "build" },
];

const quiet = (commands: Command[]) => commands.map((c) => `${c.group}:${c.title}`);

test("the placeholder asks for the workspace when someone is signed in", () => {
  assert.equal(palettePlaceholder(true), PALETTE_PLACEHOLDER_IN);
  assert.equal(palettePlaceholder(false), PALETTE_PLACEHOLDER_OUT);
  assert.match(PALETTE_PLACEHOLDER_IN, /tasks, messages, people/);
});

test("an empty query is only the static commands", () => {
  const commands = routeCommands(() => {});
  const shown = paletteItems({
    commands,
    query: "  ",
    channels,
    people,
    tasks,
    hits: [{ thread_id: "t9", snippet: "hello there", message_id: "m1" }],
    me: "me",
    jumpChannel: () => {},
    openPerson: () => {},
    openSelf: () => {},
    openThread: () => {},
  });
  assert.deepEqual(shown.map((c) => c.id), commands.map((c) => c.id));
});

test("a channel matches with or without a hash, and a person with or without @", () => {
  const jumped: string[] = [];
  const opened: string[] = [];
  const self: string[] = [];
  const run = (query: string) =>
    paletteItems({
      commands: routeCommands(() => {}),
      query,
      channels,
      people,
      tasks,
      hits: [],
      me: "me",
      jumpChannel: (id) => jumped.push(id),
      openPerson: (id) => opened.push(id),
      openSelf: () => self.push("self"),
      openThread: () => {},
    });
  const hashed = run("#ops");
  assert.deepEqual(quiet(hashed), ["Channels:#ops"]);
  hashed[0].run();
  assert.deepEqual(jumped, ["c-ops"]);
  assert.ok(run("#").some((c) => c.title === "#general"));
  assert.equal(run("#").filter((c) => c.group === "People").length, 0);

  const byHandle = run("@deployer");
  assert.equal(byHandle.find((c) => c.group === "People")?.title, "Deployer");
  assert.match(byHandle.find((c) => c.group === "People")?.hint ?? "", /@deployer/);
  byHandle.find((c) => c.id === "pe-ag")?.run();
  assert.deepEqual(opened, ["ag"]);

  const mine = run("operator");
  assert.equal(mine.find((c) => c.id === "pe-me")?.hint?.startsWith("You"), true);
  mine.find((c) => c.id === "pe-me")?.run();
  assert.deepEqual(self, ["self"]);
  assert.ok(run("@").some((c) => c.id === "pe-ag"));
  assert.equal(run("@").filter((c) => c.group === "Channels").length, 0);
});

test("task titles and message hits share one group and open the thread", () => {
  const opened: string[] = [];
  const hits: SearchHit[] = [
    { message_id: "m1", thread_id: "t1", snippet: "Deploy v9 is ready" },
    { message_id: "m2", thread_id: "t9", body: "result attached for the review", snippet: "" },
    { message_id: "m3", thread_id: null, snippet: "no thread" },
  ];
  const shown = paletteItems({
    commands: routeCommands(() => {}),
    query: "deploy",
    channels,
    people,
    tasks,
    hits: [hits[0]],
    me: "me",
    jumpChannel: () => {},
    openPerson: () => {},
    openSelf: () => {},
    openThread: (id) => opened.push(id),
  });
  const found = shown.filter((c) => c.group === "Tasks & messages");
  assert.deepEqual(found.map((c) => c.title), ["Deploy v9"]);
  found[0].run();
  assert.deepEqual(opened, ["t1"]);

  const messages = paletteItems({
    commands: [],
    query: "result attached",
    channels: [],
    people: [],
    tasks: [],
    hits: [hits[1], hits[2]],
    me: null,
    jumpChannel: () => {},
    openPerson: () => {},
    openSelf: () => {},
    openThread: (id) => opened.push(id),
  });
  assert.deepEqual(messages.map((c) => c.group), ["Messages"]);
  assert.match(messages[0].title, /result attached/);
  messages[0].run();
  assert.deepEqual(opened, ["t1", "t9"]);
  assert.equal(openThreadHash("a/b"), "#/threads/t/a%2Fb");

  const channelOnly = paletteItems({
    commands: [],
    query: "#build",
    channels: [{ id: "c", name: "build" }],
    people: [],
    tasks: [{ id: "t", title: "Open: nobody holds this", channelName: "build" }],
    hits: [],
    me: null,
    jumpChannel: () => {},
    openPerson: () => {},
    openSelf: () => {},
    openThread: () => {},
  });
  assert.deepEqual(channelOnly.map((c) => c.title), ["#build"]);
  assert.equal(channelOnly[0]?.hint, undefined);
});

test("one letter does not search, and a search in flight is not an empty result", () => {
  const shown = paletteItems({
    commands: routeCommands(() => {}),
    query: "d",
    channels,
    people,
    tasks,
    hits: [{ thread_id: "t9", snippet: "deploy note", message_id: "m" }],
    me: null,
    jumpChannel: () => {},
    openPerson: () => {},
    openSelf: () => {},
    openThread: () => {},
  });
  assert.equal(shown.some((c) => c.group === "Messages" || c.group === "Tasks & messages"), false);
  assert.equal(PALETTE_SEARCH_MIN, 2);
  assert.equal(paletteEmpty("d", 0, false), "none");
  assert.equal(paletteEmpty("de", 0, true), "searching");
  assert.equal(paletteEmpty("de", 1, true), "hidden");
  assert.equal(paletteEmpty("  ", 0, true), "hidden");
});

test("a message with no thread is dropped, and a second hit in the same thread is one row", () => {
  const rows = messageCommands(
    [
      { thread_id: "t9", message_id: "m1", snippet: "one" },
      { thread_id: "t9", message_id: "m2", snippet: "two" },
      { snippet: "orphan" },
    ],
    () => {},
  );
  assert.deepEqual(rows.map((c) => c.id), ["msg-m1"]);
});

test("the index keeps a workspace's tasks and skips a channel that fails", async () => {
  clearPaletteIndex();
  assert.equal(peekPaletteIndex("w"), null);
  let directory = false;
  const index = await loadPaletteIndex("w", {
    channels: async () => [{ id: "c1", name: "ops" }, { id: "c2", name: "quiet" }, { id: "c3", name: "  " }],
    people: async () => [{ id: "m1", name: "Rae", handle: "rae", kind: "human" }],
    tasks: async (id) => {
      if (id === "c2") throw new Error("nope");
      return id === "c1" ? [{ id: "t1", title: "Deploy v9" }, { id: "t0", title: "  " }] : [];
    },
  }, () => {
    directory = true;
  });
  assert.equal(directory, true);
  assert.deepEqual(index.tasks, [{ id: "t1", title: "Deploy v9", channelName: "ops" }]);
  assert.equal(index.people[0]?.handle, "rae");
  storePaletteIndex(index);
  assert.equal(peekPaletteIndex("w")?.tasks.length, 1);
  assert.equal(peekPaletteIndex("other"), null);
  const failed = await loadPaletteIndex("w2", {
    channels: async () => { throw new Error("down"); },
    people: async () => { throw new Error("down"); },
    tasks: async () => [{ id: "t", title: "x" }],
  });
  assert.deepEqual(failed.channels, []);
  assert.deepEqual(failed.people, []);
  clearPaletteIndex();
});

test("one channel is a label and any other count keeps the menu", () => {
  assert.equal(channelControl(1), "calm");
  assert.equal(channelControl(0), "menu");
  assert.equal(channelControl(2), "menu");
});

test("the theme commands are found by a search, not only on an empty palette", () => {
  const theme = (id: string, title: string) => ({ id, title, group: "Appearance" as const, run: () => {} });
  const commands = [...routeCommands(() => {}), theme("theme-light", "Light theme"), theme("theme-system", "Match the system theme")];
  const run = (query: string) =>
    paletteItems({
      commands,
      query,
      channels: [],
      people: [],
      tasks: [],
      hits: [],
      me: "me",
      jumpChannel: () => {},
      openPerson: () => {},
      openSelf: () => {},
      openThread: () => {},
    }).map((c) => c.id);
  assert.ok(run("light theme").includes("theme-light"));
  assert.ok(run("system").includes("theme-system"));
});
