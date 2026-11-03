import assert from "node:assert/strict";
import { test } from "node:test";
import { fileKind, fileName, filterFiles, gatherFiles, textPreviewable } from "../src/core/files.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);

test("a file linked to two tasks shows once with both sources, newest first", () => {
  const rows = gatherFiles(
    [{ id: "t1", title: "Fix login" }, { id: "t2", title: null }],
    new Map([["t1", [A, B]], ["t2", [A]]]),
    new Map([[A, { filename: "log.txt", created_at: "2026-11-01" }], [B, { filename: "shot.png", created_at: "2026-11-02" }]]),
  );
  assert.deepEqual(rows.map((r) => r.sha), [B, A]);
  assert.deepEqual(rows[1].sources, [{ id: "t1", title: "Fix login" }, { id: "t2", title: "Untitled task" }]);
});

test("a file with no meta is named by its sha", () => {
  assert.equal(fileName({ sha: A, meta: null }), "aaaaaaaaaaaa…");
});

test("kinds come from the media type", () => {
  assert.equal(fileKind("image/png"), "image");
  assert.equal(fileKind("text/plain; charset=utf-8"), "text");
  assert.equal(fileKind("application/json"), "data");
  assert.equal(fileKind("application/zip"), "archive");
  assert.equal(fileKind(null), "other");
});

test("search matches name, sha prefix and source task", () => {
  const rows = [{ sha: A, meta: { filename: "log.txt" }, sources: [{ id: "t", title: "Deploy" }] }];
  assert.equal(filterFiles(rows, "LOG").length, 1);
  assert.equal(filterFiles(rows, "aaaa").length, 1);
  assert.equal(filterFiles(rows, "deploy").length, 1);
  assert.equal(filterFiles(rows, "zzz").length, 0);
});

test("only small text previews inline", () => {
  assert.equal(textPreviewable({ mime_type: "text/plain", size_bytes: 10 }), true);
  assert.equal(textPreviewable({ mime_type: "text/plain", size_bytes: 10_000_000 }), false);
  assert.equal(textPreviewable({ mime_type: "image/png", size_bytes: 10 }), false);
});

test("a file stored with no media type is read by its extension", async () => {
  const { mediaType } = await import("../src/core/files.ts");
  assert.equal(mediaType({ mime_type: null, filename: "shot.PNG" }), "image/png");
  assert.equal(mediaType({ mime_type: "text/csv", filename: "x.png" }), "text/csv");
});
