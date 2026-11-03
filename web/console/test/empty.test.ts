import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// "Nothing here yet" reads the same everywhere: the shared EmptyState with a title and a line.
const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");

test("Board, Threads, People, Activity and Files say empty with the shared EmptyState", () => {
  for (const p of ["board/Board.tsx", "threads/Threads.tsx", "people/People.tsx", "activity/Activity.tsx", "artifacts/Library.tsx"]) assert.match(src(p), /<EmptyState title=/, p);
});

test("no screen falls back to a bare muted line for its empty list", () => {
  assert.doesNotMatch(src("people/People.tsx"), /"No members yet\."/);
  assert.doesNotMatch(src("activity/Activity.tsx"), /"No events yet\."/);
  assert.doesNotMatch(src("threads/Threads.tsx"), /className="muted conv-empty"/);
});
