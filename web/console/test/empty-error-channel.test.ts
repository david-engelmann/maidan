import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { emptyStateTitle, loadErrorMessage, channelPickView } from "../src/shell/emptyErrorChannel.ts";

const src = (p: string) => readFileSync(new URL(`../src/${p}`, import.meta.url), "utf8");

test("an empty or blank EmptyState title falls back to a calm default", () => {
  assert.equal(emptyStateTitle(""), "Nothing here yet");
  assert.equal(emptyStateTitle("   "), "Nothing here yet");
  assert.equal(emptyStateTitle(" No files yet "), "No files yet");
});

test("EmptyState announces politely and labels itself from the title", () => {
  const text = src("shell/EmptyState.tsx");
  assert.match(text, /role="status"/);
  assert.match(text, /aria-live="polite"/);
  assert.match(text, /aria-label=\{heading\}/);
});

test("a blank LoadError message falls back without echoing emptiness", () => {
  assert.equal(loadErrorMessage(""), "Could not load this screen.");
  assert.equal(loadErrorMessage(" \n\t "), "Could not load this screen.");
  assert.equal(loadErrorMessage(" Board failed "), "Board failed");
});

test("LoadError is assertive and names the retry against the message", () => {
  const text = src("shell/LoadError.tsx");
  assert.match(text, /role="alert"/);
  assert.match(text, /aria-live="assertive"/);
  assert.match(text, /aria-label=\{`Try again: \$\{text\}`\}/);
});

test("zero channels is a calm empty label, not a disabled blank select", () => {
  assert.deepEqual(channelPickView([], null), { kind: "empty" });
  assert.deepEqual(channelPickView([], "ch_old"), { kind: "empty" });
  const text = src("shell/ChannelPick.tsx");
  assert.match(text, /No channels yet/);
  assert.doesNotMatch(text, /disabled=\{!channels\.length\}/);
});

test("one channel stays a label; many keep a menu", () => {
  assert.deepEqual(channelPickView([{ id: "c1", name: "general" }], "c1"), { kind: "only", name: "general" });
  assert.deepEqual(channelPickView([{ id: "c1", name: "  " }], null), { kind: "only", name: "channel" });
  assert.deepEqual(
    channelPickView(
      [
        { id: "c1", name: "a" },
        { id: "c2", name: "b" },
      ],
      "c2",
    ),
    { kind: "menu", value: "c2", orphan: false },
  );
});

test("a remembered channel id that left the list clears the select as an orphan", () => {
  assert.deepEqual(
    channelPickView(
      [
        { id: "c1", name: "a" },
        { id: "c2", name: "b" },
      ],
      "gone",
    ),
    { kind: "menu", value: "", orphan: true },
  );
  const text = src("shell/ChannelPick.tsx");
  assert.match(text, /Select a channel/);
  assert.match(text, /aria-invalid=\{view\.orphan \|\| undefined\}/);
});

test("useSession supplies a getServerSnapshot so the store does not warn", () => {
  const text = src("hooks/useSession.ts");
  assert.match(text, /useSyncExternalStore\(store\.subscribe, store\.getSnapshot, store\.getSnapshot\)/);
});

test("main says so when #root is missing instead of mounting nothing silently", () => {
  const text = src("main.tsx");
  assert.match(text, /#root is missing/);
  assert.match(text, /console\.error/);
});

test("an Icon can take a label when it is not decorative", () => {
  const text = src("shell/icons.tsx");
  assert.match(text, /label\?: string/);
  assert.match(text, /role=\{labelled \? "img" : undefined\}/);
});
