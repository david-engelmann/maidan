import { test } from "node:test";
import assert from "node:assert/strict";
import { goSequence, isTyping, rank, requestCompose, routeCommands, score, takeCompose, type Command } from "../src/shell/commands.ts";

const cmd = (id: string, title: string, hint?: string): Command => ({ id, title, hint, group: "Actions", run: () => {} });

test("every place is in the palette with its G-letter jump", () => {
  const went: string[] = [];
  const cmds = routeCommands((id) => went.push(id));
  assert.deepEqual(cmds.map((c) => c.title), ["Board", "Needs you", "Threads", "People", "Tools", "Files"]);
  assert.deepEqual(cmds.find((c) => c.title === "Tools")?.keys, ["G", "O"]);
  cmds[2].run();
  assert.deepEqual(went, ["threads"]);
});

test("typing ranks a prefix above a word start above a scattered match", () => {
  const cmds = [cmd("a", "Sign out"), cmd("b", "New message"), cmd("c", "Needs you"), cmd("d", "Refresh this screen")];
  assert.deepEqual(rank(cmds, "ne").map((c) => c.id), ["c", "b"]);
  assert.deepEqual(rank(cmds, "rts").map((c) => c.id), ["d"]);
  assert.deepEqual(rank(cmds, "zzz"), []);
  assert.equal(rank(cmds, "  ").length, 4, "an empty query lists everything in order");
});

test("the hint matches too, below any title match", () => {
  const cmds = [cmd("files", "Files", "upload an artifact"), cmd("up", "Upload a file")];
  assert.deepEqual(rank(cmds, "upload").map((c) => c.id), ["up", "files"]);
  assert.equal(score("Board", "x"), null);
});

test("g then a letter jumps; a slow or wrong second key does not", () => {
  let s = goSequence(null, "g", 0);
  assert.equal(s.go, null);
  assert.equal(goSequence(s.state, "t", 500).go, "threads");
  assert.equal(goSequence(s.state, "t", 5000).go, null);
  assert.equal(goSequence(s.state, "x", 100).go, null);
  s = goSequence(null, "t", 0);
  assert.equal(s.go, null, "a lone letter is just a letter");
});

test("shortcuts stay out of fields", () => {
  assert.equal(isTyping({ tagName: "INPUT" } as unknown as EventTarget), true);
  assert.equal(isTyping({ tagName: "DIV", isContentEditable: true } as unknown as EventTarget), true);
  assert.equal(isTyping({ tagName: "BUTTON" } as unknown as EventTarget), false);
  assert.equal(isTyping(null), false);
});

test("New message is held for Threads until it mounts, then taken once", () => {
  const seen: string[] = [];
  (globalThis as { window?: unknown }).window = { dispatchEvent: (e: Event) => seen.push(e.type) };
  assert.equal(takeCompose(), false);
  requestCompose();
  assert.deepEqual(seen, ["maidan:compose"]);
  assert.equal(takeCompose(), true);
  assert.equal(takeCompose(), false);
});

test("j and k step through a list and stop at the ends", async () => {
  const { listStep } = await import("../src/shell/commands.ts");
  assert.equal(listStep(-1, 3, "j"), 0);
  assert.equal(listStep(-1, 3, "k"), 2);
  assert.equal(listStep(2, 3, "j"), 2);
  assert.equal(listStep(1, 3, "k"), 0);
  assert.equal(listStep(0, 0, "j"), null);
  assert.equal(listStep(0, 3, "x"), null);
});

test("a scattered title match still outranks an exact hint-only match", () => {
  const cmds = [cmd("a", "Refresh", "upload a file"), cmd("b", "Upload a file again")];
  assert.deepEqual(rank([cmd("h", "Something", "fil"), cmd("t", "Final list")], "fil").map((c) => c.id), ["t", "h"]);
  assert.deepEqual(rank(cmds, "upload").map((c) => c.id), ["b", "a"]);
});
