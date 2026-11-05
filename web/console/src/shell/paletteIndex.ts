// What the palette offers beyond the static commands: channels, people, task
// titles and message hits. Plain data in, so the ranking can be tested
// without a browser. The shell caches one workspace's index and refreshes it.
import type { SearchHit } from "../core/tools.ts";
import { rank, type Command } from "./commands.ts";

export const PALETTE_SEARCH_MIN = 2;
export const PALETTE_SEARCH_DEBOUNCE_MS = 150;

export const PALETTE_PLACEHOLDER_IN = "Search tasks, messages, people, or commands…";
export const PALETTE_PLACEHOLDER_OUT = "Go to a place or run a command…";

export function palettePlaceholder(signedIn: boolean): string {
  return signedIn ? PALETTE_PLACEHOLDER_IN : PALETTE_PLACEHOLDER_OUT;
}

/** Nothing to say until there is a query with no rows. A search in flight
 *  must not flash "nothing matches" in the gap before the answer. */
export function paletteEmpty(query: string, shown: number, searching: boolean): "searching" | "none" | "hidden" {
  if (shown > 0 || !query.trim()) return "hidden";
  return searching ? "searching" : "none";
}

export function openThreadHash(threadId: string): string {
  return `#/threads/t/${encodeURIComponent(threadId)}`;
}

export type PaletteChannel = { id: string; name: string };
export type PalettePerson = { id: string; name: string; handle?: string; kind?: string };
export type PaletteTask = { id: string; title: string; channelName?: string };

export type PaletteIndex = {
  workspaceId: string;
  channels: PaletteChannel[];
  people: PalettePerson[];
  tasks: PaletteTask[];
};

export type PaletteReaders = {
  channels: () => Promise<PaletteChannel[]>;
  people: () => Promise<PalettePerson[]>;
  tasks: (channelId: string) => Promise<Array<{ id: string; title: string }>>;
};

/** Channels, people and task titles. A failed read is an empty list, so one
 *  refusal does not blank the rest of the palette. `onDirectory` fires once
 *  channels and people are in, before task titles, so a jump does not wait
 *  on every channel's threads. */
export async function loadPaletteIndex(workspaceId: string, read: PaletteReaders, onDirectory?: (index: PaletteIndex) => void): Promise<PaletteIndex> {
  const [channels, people] = await Promise.all([
    read.channels().catch(() => [] as PaletteChannel[]),
    read.people().catch(() => [] as PalettePerson[]),
  ]);
  onDirectory?.({ workspaceId, channels, people, tasks: [] });
  const lists = await Promise.all(
    channels.map(async (c) => {
      try {
        const tasks = await read.tasks(c.id);
        return tasks.filter((t) => t.title.trim()).map((t) => ({ id: t.id, title: t.title.trim(), channelName: c.name }));
      } catch {
        return [];
      }
    }),
  );
  return { workspaceId, channels, people, tasks: lists.flat() };
}

let cached: PaletteIndex | null = null;

export function peekPaletteIndex(workspaceId: string): PaletteIndex | null {
  return cached?.workspaceId === workspaceId ? cached : null;
}

export function storePaletteIndex(index: PaletteIndex): void {
  cached = index;
}

export function clearPaletteIndex(): void {
  cached = null;
}

function snippetTitle(hit: SearchHit): string {
  const text = (hit.snippet || hit.body || "").replace(/\s+/g, " ").trim();
  if (!text) return "Message";
  return text.length > 80 ? `${text.slice(0, 79)}…` : text;
}

export function channelCommands(channels: PaletteChannel[], jump: (id: string) => void): Command[] {
  return channels
    .filter((c) => c.name.trim())
    .map((c) => ({
      id: `ch-${c.id}`,
      title: `#${c.name}`,
      group: "Channels",
      run: () => jump(c.id),
    }));
}

export function peopleCommands(people: PalettePerson[], me: string | null, openPerson: (id: string) => void, openSelf: () => void): Command[] {
  return people.map((p) => {
    const handle = (p.handle ?? "").trim().replace(/^@/, "");
    const self = me != null && p.id === me;
    const kind = p.kind === "agent" ? "Agent" : "Person";
    const hint = [self ? "You" : null, handle ? `@${handle}` : null, self ? null : kind].filter(Boolean).join(" · ");
    return {
      id: `pe-${p.id}`,
      title: p.name.trim() || (handle ? `@${handle}` : p.id.slice(0, 8)),
      hint: hint || undefined,
      group: "People",
      run: () => (self ? openSelf() : openPerson(p.id)),
    };
  });
}

export function taskCommands(tasks: PaletteTask[], openThread: (id: string) => void): Command[] {
  return tasks
    .filter((t) => t.title.trim())
    .map((t) => ({
      id: `task-${t.id}`,
      title: t.title.trim(),
      hint: t.channelName ? `#${t.channelName}` : "Task",
      group: "Tasks & messages" as const,
      run: () => openThread(t.id),
    }));
}

/** One row per thread. Threads already listed as tasks are left to that row. */
export function messageCommands(hits: SearchHit[], openThread: (id: string) => void, skipThreads?: ReadonlySet<string>): Command[] {
  const seen = new Set(skipThreads);
  const out: Command[] = [];
  for (const h of hits) {
    const threadId = h.thread_id ?? "";
    if (!threadId || seen.has(threadId)) continue;
    seen.add(threadId);
    out.push({
      id: `msg-${h.message_id || h.id || threadId}`,
      title: snippetTitle(h),
      hint: "Message",
      group: "Messages",
      run: () => openThread(threadId),
    });
  }
  return out;
}

/** A leading # or @ is how someone names a channel or a person, so the rank
 *  sees the name. A lone mark lists that whole group. */
function matchNamed(commands: Command[], query: string, strip: RegExp): Command[] {
  const q = query.trim();
  if (!q) return [];
  const needle = q.replace(strip, "").trim();
  if (!needle && strip.test(q)) return commands;
  return rank(commands, needle || q);
}

function byGroup(commands: Command[], query: string, group: Command["group"]): Command[] {
  return rank(commands.filter((c) => c.group === group), query);
}

/** Rank on the title only. A task's channel hint would otherwise list every
 *  task in #build when someone typed the channel. */
function rankTitles(commands: Command[], query: string): Command[] {
  const byId = new Map(commands.map((c) => [c.id, c]));
  return rank(commands.map((c) => ({ ...c, hint: undefined })), query).map((c) => byId.get(c.id) ?? c);
}

export function paletteItems(input: {
  commands: Command[];
  query: string;
  channels: PaletteChannel[];
  people: PalettePerson[];
  tasks: PaletteTask[];
  hits: SearchHit[];
  me: string | null;
  jumpChannel: (id: string) => void;
  openPerson: (id: string) => void;
  openSelf: () => void;
  openThread: (id: string) => void;
}): Command[] {
  const q = input.query.trim();
  if (!q) return input.commands;
  const tasks = q.length >= PALETTE_SEARCH_MIN ? rankTitles(taskCommands(input.tasks, input.openThread), q) : [];
  const foundGroup: Command["group"] = tasks.length ? "Tasks & messages" : "Messages";
  const messages = q.length >= PALETTE_SEARCH_MIN ? messageCommands(input.hits, input.openThread, new Set(tasks.map((c) => c.id.slice("task-".length)))) : [];
  const found = [...tasks, ...messages].map((c) => ({ ...c, group: foundGroup }));
  return [
    ...byGroup(input.commands, q, "Go to"),
    ...byGroup(input.commands, q, "Actions"),
    ...matchNamed(channelCommands(input.channels, input.jumpChannel), q, /^#+/),
    ...matchNamed(peopleCommands(input.people, input.me, input.openPerson, input.openSelf), q, /^@+/),
    ...found,
  ];
}
