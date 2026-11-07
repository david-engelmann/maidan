import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// "Nothing here yet" reads the same everywhere: the shared EmptyState with a title and a line.
const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");

test("Board, Threads, People, Activity, Catalog and Files say empty with a title and a body", () => {
  for (const p of ["board/Board.tsx", "threads/Threads.tsx", "people/People.tsx", "activity/Activity.tsx", "catalog/Catalog.tsx", "artifacts/Library.tsx"]) {
    const blocks = src(p).match(/<EmptyState\b[\s\S]*?\/>/g) ?? [];
    assert.ok(blocks.length > 0, p);
    for (const block of blocks) {
      assert.match(block, /\btitle=/, p);
      assert.match(block, /\bbody=/, p);
      assert.doesNotMatch(block, /\bbody=(?:""|''|\{\s*""\s*\}|\{\s*''\s*\}|\{\s*`\s*`\s*\})/, p);
    }
  }
});

test("no screen falls back to a bare muted line for its empty list", () => {
  assert.doesNotMatch(src("people/People.tsx"), /"No members yet\."/);
  assert.doesNotMatch(src("activity/Activity.tsx"), /"No events yet\."/);
  assert.doesNotMatch(src("threads/Threads.tsx"), /className="muted conv-empty"/);
});
