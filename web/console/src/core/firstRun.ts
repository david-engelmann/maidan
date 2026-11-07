// What a brand-new user sees: signed out, then a workspace with no channel
// and no task yet. Kept apart from the screens so the copy is tested.

/** Where a channel or a task is created: Tools, then Workspace. */
export const NEW_WORK_HREF = "#/tools/workspace";

/** The board and thread list line when a channel shows no task. */
export function emptyChannelText(channels: number): string {
  return channels > 0 ? "No tasks in this channel yet." : "No channels yet. Create one under Tools, then Workspace.";
}

/** The top bar's title: signed out, every place shows the sign-in card. */
export function shellTitle(signedOut: boolean, placeTitle: string): string {
  return signedOut ? "Sign in" : placeTitle;
}

export type LiveKind = "connected" | "reconnecting" | "refused" | "idle" | "offline" | string;

/** The live pill's word. Signed out is not offline: the server may be fine. */
export function liveWord(kind: LiveKind, signedOut: boolean): string {
  if (signedOut) return "Signed out";
  if (kind === "connected") return "Live";
  if (kind === "reconnecting") return "Reconnecting";
  if (kind === "refused") return "Refused";
  if (kind === "connecting") return "Connecting";
  return "Offline";
}

/**
 * The sign-in card's line for someone with no token. A person signs in with
 * their identity provider and adds agents from the board or People; a token
 * is for an agent, not for them.
 */
export function noTokenHelp(oidc: boolean): string {
  return oidc
    ? "No token? You don't need one. Sign in, then add your agents from the board or People; each gets its own token."
    : "No token? Ask a workspace admin for one.";
}

/** Said when an identity signed in but belongs to no workspace here. */
export function noWorkspaceText(signup: boolean): string {
  const lead = "You signed in, but that identity isn't a member of any workspace on this server, so no session was made.";
  return signup
    ? `${lead} Ask a workspace admin to invite you, or create your own workspace with an invite.`
    : `${lead} Ask a workspace admin to invite you.`;
}

const FIRST_RUN_KEY = "maidan_console_first_run_dismissed";

/** Whether the first-run guide was put away for this workspace. Blocked storage reads as not dismissed. */
export function firstRunDismissed(workspace: string | null, storage: Pick<Storage, "getItem"> | null = safeStorage()): boolean {
  if (!workspace || !storage) return false;
  try {
    return (storage.getItem(FIRST_RUN_KEY) ?? "").split(",").includes(workspace);
  } catch {
    return false;
  }
}

/** Put the guide away for this workspace. The page hides it either way; storage only makes it stick. */
export function dismissFirstRun(workspace: string | null, storage: Pick<Storage, "getItem" | "setItem"> | null = safeStorage()): void {
  if (!workspace || !storage) return;
  try {
    const kept = (storage.getItem(FIRST_RUN_KEY) ?? "").split(",").filter(Boolean);
    if (!kept.includes(workspace)) storage.setItem(FIRST_RUN_KEY, [...kept, workspace].join(","));
  } catch {
    /* storage full or blocked: the guide stays hidden for this page only */
  }
}

/** The guide shows on a board that has loaded and holds no task of the person's yet. */
export function showFirstRun(status: string, tasks: number, dismissed: boolean): boolean {
  return status === "ready" && tasks === 0 && !dismissed;
}

/** The title of the task a workspace made by sign-up opens with (the server's `STARTER_TASK_TITLE`). */
export const STARTER_TASK_TITLE = "Say hello";

/**
 * The tasks that put the guide away. The starter task does not: it is the
 * guide's own second step, so it counts only once an agent has picked it up
 * or it has moved on from open.
 */
export function guideTasks(threads: ReadonlyArray<{ title?: string | null; state?: string; assignee_id?: string | null }>): number {
  return threads.filter((t) => !(t.title === STARTER_TASK_TITLE && t.state === "open" && !t.assignee_id)).length;
}

/** What to tell the agent once it is connected. */
export function askAgentLine(channel: string | null): string {
  return `Ask your agent: take the next task in #${channel ?? "general"}.`;
}

function safeStorage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/**
 * Whether this origin serves a support page. A self-hosted server has none,
 * and a link to its 404 is worse than no link, so the page is probed once.
 */
export async function hasSupportPage(base: string, fetchFn: (url: string, init?: RequestInit) => Promise<Response> = fetch): Promise<boolean> {
  try {
    const res = await fetchFn(supportHref(base), { method: "HEAD", credentials: "omit", signal: AbortSignal.timeout(4000) });
    return res.ok && (res.headers.get("content-type") ?? "").includes("text/html");
  } catch {
    return false;
  }
}

export function supportHref(base: string): string {
  return `${base.replace(/\/$/, "")}/support`;
}
