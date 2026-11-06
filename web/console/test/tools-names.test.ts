import assert from "node:assert/strict";
import { test } from "node:test";
import {
  NOTIFICATION_KINDS,
  describeNotificationKind,
  taskFollowLabel,
  tasksToFollow,
  threadLinkLabel,
  type PickerTask,
} from "../src/core/tools.ts";

const tasks: PickerTask[] = [
  { id: "b", title: "Beta", channelId: "ops", channelName: "  ops  " },
  { id: "a", title: "Alpha", channelName: "general" },
  { id: "c", title: "Alpha", channelId: "gen", channelName: "general" },
  { id: "d", title: "   ", channelName: "general" },
  { id: "e", title: "Whisper", channelId: "dm", channelName: null },
  { id: "f", title: "Hello", channelName: "  " },
];

test("a followed task is the channel and the title, or the title alone", () => {
  const map = new Map(tasks.map((task) => [task.id, task]));
  assert.deepEqual(taskFollowLabel("a", map), { text: "#general \u2014 Alpha", resolved: true });
  assert.deepEqual(taskFollowLabel("a", tasks), { text: "#general \u2014 Alpha", resolved: true });
  assert.deepEqual(taskFollowLabel("b", tasks), { text: "#ops \u2014 Beta", resolved: true });
  assert.deepEqual(taskFollowLabel("e", map), { text: "Whisper", resolved: true });
  assert.deepEqual(taskFollowLabel("f", tasks), { text: "Hello", resolved: true });
  assert.deepEqual(taskFollowLabel("d", map), { text: "d", resolved: false });
  assert.deepEqual(taskFollowLabel("missing", tasks), { text: "missing", resolved: false });
});

test("tasks still to follow drop blanks and ones already followed, and sort by channel then title", () => {
  assert.deepEqual(tasksToFollow(tasks, new Set(["b"])), [
    { id: "f", label: "Hello" },
    { id: "e", label: "Whisper" },
    { id: "a", label: "#general \u2014 Alpha" },
    { id: "c", label: "#general \u2014 Alpha" },
  ]);
  assert.deepEqual(tasksToFollow(tasks, tasks.map((task) => task.id)), []);
});

test("a thread link names the channel and the title, or keeps the fallback", () => {
  const map = new Map(tasks.map((task) => [task.id, task]));
  assert.equal(threadLinkLabel("c", map, "Open the thread"), "#general \u00b7 Alpha");
  assert.equal(threadLinkLabel("e", map, "Open the thread"), "Whisper");
  assert.equal(threadLinkLabel("f", map, "Open the thread"), "Hello");
  assert.equal(threadLinkLabel("d", map, "thread dddddddd"), "thread dddddddd");
  assert.equal(threadLinkLabel("nope", map, "Open the thread"), "Open the thread");
  assert.equal(threadLinkLabel("nope", new Map(), "thread abcdef01"), "thread abcdef01");
});

test("notification kinds are words, and an unknown kind is a capitalized phrase", () => {
  assert.equal(describeNotificationKind("message_posted"), "Message posted");
  assert.equal(describeNotificationKind("mention_recorded"), "Mentioned you");
  assert.equal(describeNotificationKind("thread_ready"), "Task ready");
  assert.equal(describeNotificationKind("thread_result_set"), "Result posted");
  assert.equal(describeNotificationKind("review_submitted"), "Review submitted");
  assert.equal(describeNotificationKind("claim_expired"), "Claim expired");
  assert.equal(describeNotificationKind("thread_landed"), "Task landed");
  assert.equal(describeNotificationKind("approval_requested"), "Approval requested");
  assert.equal(describeNotificationKind("thread_assigned"), "Assigned to you");
  assert.equal(describeNotificationKind("thread_assignment_changed"), "Assigned to you");
  assert.equal(describeNotificationKind("custom_kind"), "Custom kind");
  assert.equal(describeNotificationKind("CUSTOM_KIND"), "Custom kind");
  assert.equal(describeNotificationKind("thread_created"), "Thread created");
  assert.equal(describeNotificationKind(""), "");
  for (const kind of NOTIFICATION_KINDS) {
    const label = describeNotificationKind(kind);
    assert.notEqual(label, kind);
    assert.notEqual(label, kind.replaceAll("_", " "));
    assert.equal(label[0], label[0].toUpperCase());
  }
});
