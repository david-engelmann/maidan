import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import type { Thread } from "../src/core/board.ts";
import {
  NO_SEATS,
  PeopleApi,
  PeopleError,
  REPO_REQUIRED,
  applyPresence,
  circuit,
  circuitStatus,
  filterRoster,
  holding,
  loadSeats,
  nextWell,
  roster,
  saveSeats,
  seat,
  seatRefusal,
  tokenState,
  unseat,
  type Person,
} from "../src/core/people.ts";
import { parseRoute } from "../src/shell/router.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };

function fake(responses: Array<{ status?: number; body?: unknown }>, creds: Credentials = session) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  let i = 0;
  const api = new PeopleApi(
    async (url, init) => {
      calls.push({ url, init });
      const r = responses[Math.min(i++, responses.length - 1)] ?? {};
      return new Response(r.body === undefined ? null : JSON.stringify(r.body), { status: r.status ?? 200 });
    },
    () => creds,
    () => "session",
  );
  return { api, calls };
}

const P = (id: string, kind: string, name = id): Person => ({ id, name, handle: name.toLowerCase(), kind, createdAt: null });
const T = (o: Partial<Thread> & { id: string }) => ({ state: "open", updated_at: "2026-10-30T10:00:00Z", ...o }) as Thread;

test("people and swarm routes land on People", () => {
  assert.equal(parseRoute("#/people"), "people");
  assert.equal(parseRoute("#/people/swarm"), "people");
  assert.equal(parseRoute("#/people/m/abc"), "people");
});

test("a presence snapshot replaces, an update edits, offline removes", () => {
  const a = applyPresence(new Map(), { type: "presence_snapshot", members: [{ member_id: "a", status: "online" }, { member_id: "b", status: "away" }] } as never);
  assert.deepEqual([...(a ?? [])], [["a", "online"], ["b", "away"]]);
  const b = applyPresence(a ?? new Map(), { type: "presence", member_id: "b", status: "online" } as never);
  assert.equal(b?.get("b"), "online");
  const c = applyPresence(b ?? new Map(), { type: "presence", member_id: "a", status: "offline" } as never);
  assert.equal(c?.has("a"), false);
  assert.equal(applyPresence(new Map(), { type: "hello" } as never), null);
});

test("holding prefers the running task and skips closed ones", () => {
  const held = holding([
    T({ id: "1", assignee_id: "a", updated_at: "2026-10-30T12:00:00Z" }),
    T({ id: "2", assignee_id: "a", work_started_at: "2026-10-30T09:00:00Z", updated_at: "2026-10-30T09:00:00Z" }),
    T({ id: "3", assignee_id: "b", state: "closed" }),
  ]);
  assert.equal(held.get("a")?.id, "2");
  assert.equal(held.has("b"), false);
});

test("the roster puts live members first, agents before humans, then names", () => {
  const rows = roster([P("h1", "human", "Zed"), P("a2", "agent", "Bee"), P("a1", "agent", "Ant"), P("h2", "human", "Amy")], new Map([["h1", "online"], ["a2", "away"]]), new Map(), "h1");
  assert.deepEqual(rows.map((r) => r.person.id), ["h1", "a2", "a1", "h2"]);
  assert.equal(rows[0].me, true);
  assert.deepEqual(filterRoster(rows, " AM ").map((r) => r.person.id), ["h2"]);
});

test("token states", () => {
  const now = Date.parse("2026-10-30T12:00:00Z");
  const t = { id: "t", label: null, capabilities: [], createdAt: "2026-01-01T00:00:00Z", expiresAt: null, revokedAt: null };
  assert.equal(tokenState(t, now), "active");
  assert.equal(tokenState({ ...t, expiresAt: "2026-10-01T00:00:00Z" }, now), "expired");
  assert.equal(tokenState({ ...t, revokedAt: "2026-02-01T00:00:00Z" }, now), "revoked");
});

test("seating refuses an agent at sign-off and a coder reviewing their own work", () => {
  const ag = P("ag", "agent", "Bot");
  const hu = P("hu", "human", "Dee");
  assert.match(seatRefusal(NO_SEATS, "human", ag) ?? "", /needs a human/);
  assert.equal(seatRefusal(NO_SEATS, "human", hu), null);
  const s = seat(NO_SEATS, "coder", "ag");
  assert.match(seatRefusal(s, "reviewer", ag) ?? "", /own work/);
  assert.match(seatRefusal(seat(NO_SEATS, "reviewer", "ag"), "coder", ag) ?? "", /own work/);
  assert.match(seatRefusal(NO_SEATS, "planner", undefined) ?? "", /not in this workspace/);
});

test("one member fills one role, and the circuit reads in the landing's voice", () => {
  let s = seat(NO_SEATS, "planner", "a");
  s = seat(s, "coder", "a");
  assert.equal(s.planner, null);
  assert.equal(s.coder, "a");
  const on = () => "online" as const;
  assert.match(circuitStatus(circuit(NO_SEATS), on), /Seat a Planner/);
  assert.equal(circuitStatus(circuit(seat(NO_SEATS, "planner", "p")), on), "Coder next. The channel is cut, waiting.");
  const jam = seat(seat(NO_SEATS, "planner", "p"), "reviewer", "r");
  assert.equal(circuit(jam).jammed, true);
  assert.equal(circuitStatus(circuit(jam), on), "Jammed. No Coder, so nothing gets past the Planner.");
  const full = seat(seat(seat(seat(NO_SEATS, "planner", "p"), "coder", "c"), "reviewer", "r"), "human", "h");
  assert.equal(circuit(full).complete, true);
  assert.equal(circuitStatus(circuit(full), on), "Circuit closed. Every seat is online.");
  assert.equal(circuitStatus(circuit(full), (r) => (r === "human" ? "away" : "online")), "Circuit closed. Human is not online now.");
  assert.equal(circuit(unseat(full, "human")).placed, 3);
});

test("saved seats round-trip per workspace and drop members who left", () => {
  const box = new Map<string, string>();
  const store = { getItem: (k: string) => box.get(k) ?? null, setItem: (k: string, v: string) => void box.set(k, v) };
  saveSeats(store, "w1", seat(seat(NO_SEATS, "planner", "p"), "coder", "gone"));
  assert.deepEqual(loadSeats(store, "w1", [P("p", "agent")]), { ...NO_SEATS, planner: "p" });
  assert.deepEqual(loadSeats(store, "w2", [P("p", "agent")]), NO_SEATS);
  box.set("maidan_console_swarm_seats:w3", "{not json");
  assert.deepEqual(loadSeats(store, "w3", []), NO_SEATS);
});

test("arrow keys walk the two-by-two wells", () => {
  assert.equal(nextWell("planner", "ArrowRight"), "coder");
  assert.equal(nextWell("coder", "ArrowRight"), "planner");
  assert.equal(nextWell("planner", "ArrowDown"), "reviewer");
  assert.equal(nextWell("human", "ArrowUp"), "coder");
  assert.equal(nextWell("reviewer", "Home"), "planner");
  assert.equal(nextWell("planner", "End"), "human");
  assert.equal(nextWell("coder", "x"), "coder");
});

test("members drop tombstones and fall back to a handle for the name", async () => {
  const { api, calls } = fake([{ body: [{ id: "m1", handle: "dee", kind: "human" }, { id: "m2", display_name: "Gone", tombstoned_at: "2026-01-01T00:00:00Z" }] }]);
  const rows = await api.people("w");
  assert.deepEqual(rows.map((r) => [r.id, r.name]), [["m1", "dee"]]);
  assert.match(calls[0].url, /\/ui\/api\/workspaces\/w\/members$/);
});

test("swarm lists need a token, and are refused without one before any request", async () => {
  const { api, calls } = fake([{ body: [] }]);
  await assert.rejects(api.claims("w"), PeopleError);
  assert.equal(calls.length, 0);
  const b = fake([{ body: [{ slot: "s", in_use: 1, capacity: 2 }] }], bearer);
  assert.equal((await b.api.slots("w"))[0].slot, "s");
  assert.equal(b.calls[0].url, "http://h/workspaces/w/slots");
});

test("pausing checks the repo first and shows the server's own sentence on a conflict", async () => {
  const { api, calls } = fake([{ status: 409, body: { detail: "That repository is already paused." } }], bearer);
  await assert.rejects(api.pause("w", "nope", ""), new PeopleError(REPO_REQUIRED));
  assert.equal(calls.length, 0);
  await assert.rejects(api.pause("w", "o/r", ""), /already paused/);
  assert.equal(calls[0].init?.method, "PUT");
  assert.deepEqual(JSON.parse(String(calls[0].init?.body)), { repo: "o/r" });
  const r = fake([{ status: 204 }], bearer);
  await r.api.resume("w", "o/r");
  assert.equal(r.calls[0].url, "http://h/workspaces/w/repo-pauses?repo=o%2Fr");
  assert.equal(r.calls[0].init?.method, "DELETE");
});
