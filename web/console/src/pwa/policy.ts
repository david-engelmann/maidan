// What the console's service worker may keep offline. Only the static shell:
// the page and its pinned build files. Never an API answer, never anything
// sent with a token, and never a sign-in or socket path, so nothing that
// carries auth can be read back from the cache by a later visitor.

export const SHELL_SCOPE = "/console/";
export const SHELL_FILES = ["", "app.js", "vendor.js", "style.css", "geist.woff2", "icon.png", "icon-192.png", "icon-512.png", "manifest.webmanifest"] as const;
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
  if (u.pathname === "/console") return SHELL_SCOPE;
  if (!u.pathname.startsWith(SHELL_SCOPE)) return null;
  const name = u.pathname.slice(SHELL_SCOPE.length);
  return (SHELL_FILES as readonly string[]).includes(name) ? u.pathname : null;
}

/** May this response be stored for `key`? Only plain same-origin 200s. */
export function cacheable(status: number, type: string, cacheControl: string | null): boolean {
  if (status !== 200 || type !== "basic") return false;
  return !/no-store|private/i.test(cacheControl ?? "");
}

export type PushPayload = { title: string; body: string; kind?: string; log_id?: string };

/** Read a push message body the way the classic board's worker did. */
export function pushPayload(json: (() => unknown) | null, text: (() => string) | null): PushPayload {
  const base: PushPayload = { title: "Maidan", body: "" };
  if (!json) return base;
  try {
    const p = json() as Partial<Record<string, unknown>> | null;
    if (!p || typeof p !== "object") return base;
    return {
      title: typeof p.title === "string" && p.title ? p.title : "Maidan",
      body: typeof p.body === "string" ? p.body : "",
      kind: typeof p.kind === "string" ? p.kind : undefined,
      log_id: typeof p.log_id === "string" ? p.log_id : undefined,
    };
  } catch {
    return { ...base, body: text ? text() : "" };
  }
}

/** Where a notification click lands. */
export const NOTIFICATION_TARGET = "/console/#/needs";
