// What the console's service worker may keep offline. Only the static shell:
// the page and its pinned build files. Never an API answer, never anything
// sent with a token, and never a sign-in or socket path, so nothing that
// carries auth can be read back from the cache by a later visitor.

export const SHELL_SCOPE = "/ui/";
export const SHELL_FILES = ["", "app.js", "vendor.js", "rolldown-runtime.js", "theme.js", "style.css", "geist.woff2", "icon.png", "icon-192.png", "icon-512.png", "manifest.webmanifest"] as const;
export const SHELL_CACHE = "maidan-console-shell-v1";

const NEVER_PREFIXES = ["/ui/api", "/v1", "/auth", "/ws", "/oauth", "/.well-known", "/web-push", "/members", "/workspaces"];

export type RequestFacts = { method: string; url: string; origin: string; hasAuthorization: boolean; mode?: string };

/** The shell path a request maps to, or null when it must go to the network only. */
export function shellKey(r: RequestFacts): string | null {
  if (r.method !== "GET" || r.hasAuthorization) return null;
  let u: URL;
  try {
    u = new URL(r.url);
  } catch {
    return null;
  }
  if (u.origin !== r.origin || u.search !== "") return null;
  if (NEVER_PREFIXES.some((p) => u.pathname === p || u.pathname.startsWith(`${p}/`))) return null;
  if (u.pathname === "/ui") return SHELL_SCOPE;
  if (!u.pathname.startsWith(SHELL_SCOPE)) return null;
  const name = u.pathname.slice(SHELL_SCOPE.length);
  return (SHELL_FILES as readonly string[]).includes(name) ? u.pathname : null;
}

/** May this response be stored for `key`? Only plain same-origin 200s. */
export function cacheable(status: number, type: string, cacheControl: string | null): boolean {
  if (status !== 200 || type !== "basic") return false;
  return !/no-store|private/i.test(cacheControl ?? "");
}

export type PushPayload = { title: string; body: string; kind?: string; log_id?: string; thread_id?: string; url?: string };

const PHRASES: Record<string, string> = {
  mention_recorded: "You were mentioned in a thread.",
  approval_requested: "An approval gate is waiting on you.",
  review_submitted: "Changes were requested on your work.",
  message_posted: "New message in a followed thread.",
  thread_landed: "A followed task landed.",
  thread_result_set: "A followed member posted a result.",
  thread_state_changed: "A followed task changed state.",
  thread_assignment_changed: "Task assignment changed.",
  member_frozen: "A followed member's availability changed.",
  member_unfrozen: "A followed member's availability changed.",
};

const CLAIM = new Set(["claim_expired", "claim_stalled", "claim_unacknowledged", "claim_failed"]);

/** Inbox kinds open the inbox. Approval and claim kinds still land on Needs you. */
const INBOX_KINDS = new Set(["mention_recorded", "message_posted", "thread_landed", "thread_result_set", "thread_state_changed", "thread_assignment_changed", "member_frozen", "member_unfrozen"]);

function asId(value: unknown): string | undefined {
  if (typeof value === "string" && value.trim()) return value;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  return undefined;
}

function humanBody(body: string, kind: string | undefined): string {
  const trimmed = body.trim();
  const matched = /^New notification \(([a-z0-9_]+)\)$/.exec(trimmed);
  const snake = matched?.[1] ?? (trimmed === "" && kind ? kind : "");
  if (!snake) return body;
  if (CLAIM.has(snake)) return "A claimed task needs attention.";
  return PHRASES[snake] ?? `New notification: ${snake.replace(/_/g, " ")}`;
}

/** Read a push message body the way the classic board's worker did. `log_id` arrives as a number. */
export function pushPayload(json: (() => unknown) | null, text: (() => string) | null): PushPayload {
  const base: PushPayload = { title: "Maidan", body: "" };
  if (!json) return base;
  try {
    const p = json() as Partial<Record<string, unknown>> | null;
    if (!p || typeof p !== "object") return base;
    const kind = typeof p.kind === "string" ? p.kind : undefined;
    const out: PushPayload = {
      title: typeof p.title === "string" && p.title ? p.title : "Maidan",
      body: humanBody(typeof p.body === "string" ? p.body : "", kind),
    };
    if (kind) out.kind = kind;
    const logId = asId(p.log_id);
    if (logId) out.log_id = logId;
    if (typeof p.thread_id === "string" && p.thread_id) out.thread_id = p.thread_id;
    if (typeof p.url === "string" && p.url) out.url = p.url;
    return out;
  } catch {
    return { ...base, body: text ? text() : "" };
  }
}

/** Where a notification click lands. */
export const NOTIFICATION_TARGET = "/ui/#/needs";

function safeConsoleUrl(url: string): boolean {
  if (!url.startsWith("/ui/#/") || url.includes("//")) return false;
  for (let i = 0; i < url.length; i++) {
    const code = url.charCodeAt(i);
    if (code < 32 || code === 127) return false;
  }
  return true;
}

/** A console path for a push click: an explicit hash, the thread, the inbox, or Needs you. */
export function notificationTarget(data?: Partial<PushPayload> | null): string {
  if (data?.url && safeConsoleUrl(data.url)) return data.url;
  if (data?.thread_id && /^[A-Za-z0-9_-]+$/.test(data.thread_id)) return `/ui/#/threads/t/${data.thread_id}`;
  if (data?.kind && INBOX_KINDS.has(data.kind)) return "/ui/#/tools/inbox";
  return NOTIFICATION_TARGET;
}
