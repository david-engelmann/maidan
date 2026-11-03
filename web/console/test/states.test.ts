import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// Every screen says "could not load" the same way: the shared LoadError card
// with a Try again, shown once.
const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");
const SCREENS = ["board/Board.tsx", "needs/NeedsYou.tsx", "threads/Threads.tsx", "people/People.tsx"];

test("Board, Needs you, Threads and People show a load failure with the shared retry card", () => {
  for (const p of SCREENS) assert.match(src(p), /<LoadError [^>]*onRetry=/, p);
});

test("a bare error line is not a screen's only way to say it could not load", () => {
  assert.doesNotMatch(src("threads/Threads.tsx"), /className="conv-error pad" role="alert">\{data\.error\}/);
  assert.doesNotMatch(src("people/People.tsx"), /loaded\.value \? "stale" : "err"/);
});

test("the board names a load failure once, in the card, not also in its summary", () => {
  assert.match(src("board/Board.tsx"), /data\.status === "error" && !data\.threads\.length \? "" : data\.error/);
});

test("People loads behind skeleton rows, like Board and Threads, not a bare word", () => {
  assert.match(src("people/People.tsx"), /loaded\.status === "loading" \? \(\s*<div className="roster-sk" role="status"[^>]*>\s*<span className="skeleton row-sk"/);
});
