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
  return "Offline";
}
