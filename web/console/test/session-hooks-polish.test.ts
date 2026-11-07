import assert from "node:assert/strict";
import { test } from "node:test";
import { ControlBus } from "../src/core/bus.ts";
import { controlBus } from "../src/core/bus.ts";
import { SessionStore, type Fetch } from "../src/core/session.ts";
import { sameOriginReturn, signInReturnTo } from "../src/core/signup.ts";
import { boardDataForRoom, readStoredChannel, writeStoredChannel } from "../src/hooks/useBoard.ts";
import { rosterReloadsOn, swarmPolls } from "../src/hooks/usePeople.ts";

const BASE = "https://maidan.example";
const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

function memory(): Storage {
  const rows = new Map<string, string>();
  return {
    get length() {
      return rows.size;
    },
    clear: () => rows.clear(),
    getItem: (key: string) => rows.get(key) ?? null,
    key: (index: number) => [...rows.keys()][index] ?? null,
    removeItem: (key: string) => {
      rows.delete(key);
    },
    setItem: (key: string, value: string) => {
      rows.set(key, value);
    },
  };
}

test("signing in with another workspace's token drops the room that was pinned", async () => {
  let workspace = "ws_a";
  const fetchFn: Fetch = async (url) => {
    if (url.endsWith("/auth/session/from-token")) return json(200, {});
    if (url.endsWith("/auth/session")) return json(200, { member_id: "mem", token_id: "tok", workspace_id: workspace });
    if (url.endsWith("/me")) {
      return json(200, { member_id: "mem", workspace_id: workspace, is_bearer: true, capabilities: ["workspace:read"], known_capabilities: ["workspace:read"] });
    }
    return json(404, {});
  };
  const s = new SessionStore(BASE, fetchFn);
  await s.signInWithToken("token-a");
  assert.equal(s.getSnapshot().workspaceId, "ws_a");
  controlBus.emit({ type: "presence_snapshot", members: [{ member_id: "mem", status: "online" }] });
  assert.equal(controlBus.last?.type, "presence_snapshot");
  s.setWorkspace("ws_pinned");
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(s.getSnapshot().workspaceId, "ws_pinned");
  assert.equal(controlBus.last, null);
  assert.deepEqual(s.getSnapshot().grant?.capabilities, ["workspace:read"]);
  workspace = "ws_b";
  await s.signInWithToken("token-b");
  assert.equal(s.getSnapshot().workspaceId, "ws_b");
  assert.equal(s.getSnapshot().memberId, "mem");
  s.signOut();
  assert.equal(s.getSnapshot().workspaceId, null);
  assert.equal(s.getSnapshot().grant, null);
});

test("rotating this page's own token never blanks the room it is showing", async () => {
  const fetchFn: Fetch = async (url) => {
    if (url.endsWith("/auth/session/from-token")) return json(200, {});
    if (url.endsWith("/auth/session")) return json(200, { member_id: "mem", token_id: "tok", workspace_id: "ws_a" });
    if (url.endsWith("/me")) {
      return json(200, { member_id: "mem", workspace_id: "ws_a", is_bearer: true, capabilities: ["token:admin"], known_capabilities: ["token:admin"] });
    }
    return json(404, {});
  };
  const s = new SessionStore(BASE, fetchFn);
  await s.signInWithToken("token-a");
  const rooms: Array<string | null> = [];
  const off = s.subscribe(() => rooms.push(s.getSnapshot().workspaceId));
  assert.equal(await s.adoptRotated("token-a2", s.connection()), true);
  off();
  // A null here would unmount People, and the new secret it shows once with it.
  assert.equal(rooms.includes(null), false);
  assert.equal(s.getSnapshot().workspaceId, "ws_a");
  assert.equal(s.getSnapshot().memberId, "mem");
});

test("an expired cookie session forgets the workspace, the grant, and presence", async () => {
  let open = true;
  const fetchFn: Fetch = async (url) => {
    if (url.endsWith("/auth/session")) return open ? json(200, { member_id: "mem", token_id: null, workspace_id: "ws_a" }) : json(401, {});
    if (url === `${BASE}/ui/api/me`) {
      return json(200, { member_id: "mem", workspace_id: "ws_a", is_bearer: false, capabilities: ["workspace:read"], known_capabilities: ["workspace:read"] });
    }
    return json(404, {});
  };
  const s = new SessionStore(BASE, fetchFn);
  await s.refresh();
  assert.equal(s.getSnapshot().workspaceId, "ws_a");
  assert.ok(s.getSnapshot().grant);
  controlBus.emit({ type: "presence_snapshot", members: [] });
  open = false;
  await s.refresh();
  assert.equal(s.getSnapshot().status, "signed-out");
  assert.equal(s.getSnapshot().workspaceId, null);
  assert.equal(s.getSnapshot().mode, null);
  assert.equal(s.getSnapshot().grant, null);
  assert.equal(controlBus.last, null);
});

test("a control bus clear drops the snapshot and tells listeners", () => {
  const bus = new ControlBus();
  let cleared = 0;
  const off = bus.onClear(() => {
    cleared += 1;
  });
  bus.emit({ type: "presence_snapshot", members: [] });
  assert.equal(bus.last?.type, "presence_snapshot");
  bus.clear();
  assert.equal(bus.last, null);
  assert.equal(cleared, 1);
  off();
  bus.clear();
  assert.equal(cleared, 1);
});

test("each workspace remembers its own channel, and the legacy key still works", () => {
  const store = memory();
  assert.equal(readStoredChannel("room-a", store), null);
  writeStoredChannel("room-a", "ch_quiet", store);
  writeStoredChannel("room-b", "ch_afar", store);
  assert.equal(readStoredChannel("room-a", store), "ch_quiet");
  assert.equal(readStoredChannel("room-b", store), "ch_afar");
  assert.equal(store.getItem("maidan_console_channel"), "ch_afar");
  store.removeItem("maidan_console_channel:room-a");
  assert.equal(readStoredChannel("room-a", store), "ch_afar");
  const empty = boardDataForRoom(null);
  const loading = boardDataForRoom("room-b");
  assert.equal(empty.status, "idle");
  assert.equal(loading.status, "loading");
  assert.deepEqual(loading.channels, []);
  assert.equal(loading.channelId, null);
  assert.notEqual(empty.members, loading.members);
});

test("the roster reloads when someone joins or leaves, and the swarm poll needs a room", () => {
  assert.equal(rosterReloadsOn("member_joined"), true);
  assert.equal(rosterReloadsOn("member_removed"), true);
  assert.equal(rosterReloadsOn("member_created"), true);
  assert.equal(rosterReloadsOn("member_updated"), true);
  assert.equal(rosterReloadsOn("message_posted"), false);
  assert.equal(rosterReloadsOn(undefined), false);
  assert.equal(swarmPolls("room-a"), true);
  assert.equal(swarmPolls(null), false);
});

test("sign-in returns to the open console route and never carries a confirmation secret", () => {
  assert.equal(signInReturnTo(""), "/ui/");
  assert.equal(signInReturnTo("#/board"), "/ui/");
  assert.equal(signInReturnTo("#/needs"), "/ui/#/needs");
  assert.equal(signInReturnTo("#/people/m/mem_1"), "/ui/#/people/m/mem_1");
  assert.equal(signInReturnTo("#/board?channel=ch_1"), "/ui/#/board?channel=ch_1");
  assert.equal(signInReturnTo("#/confirm-approval=gate-1.one-time-token"), "/ui/#/needs");
  assert.equal(signInReturnTo("#confirm-approval=gate-1.one-time-token"), "/ui/#/needs");
  assert.equal(signInReturnTo("#/confirm-approval=gate-1.one-time-token").includes("one-time-token"), false);
  // The server accepts `#` in a same-origin path. The query value is encoded
  // so the browser does not treat it as this page's fragment.
  const needs = signInReturnTo("#/needs");
  assert.equal(sameOriginReturn(needs), needs);
  assert.equal(decodeURIComponent(encodeURIComponent(needs)), "/ui/#/needs");
  assert.equal(signInReturnTo("#/needs\n//evil.example"), "/ui/");
});
