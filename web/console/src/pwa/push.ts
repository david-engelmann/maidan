// Web Push for the console, on the server's existing endpoints: the VAPID
// key from /web-push/vapid-public-key and the member's
// /members/{id}/push-subscriptions. Everything browser-shaped is passed in,
// so the flow is testable without a browser.

export type PushReason =
  | "registered"
  | "no_member"
  | "unsupported"
  | "vapid_unavailable"
  | "vapid_unset"
  | "permission_denied"
  | "register_failed"
  | "off";

export type PushOutcome = { registered: boolean; reason: PushReason | string };

export type SubscriptionLike = { endpoint: string; toJSON(): { endpoint?: string; keys?: Record<string, string> }; unsubscribe(): Promise<boolean> };
export type PushManagerLike = {
  getSubscription(): Promise<SubscriptionLike | null>;
  subscribe(o: { userVisibleOnly: boolean; applicationServerKey: Uint8Array<ArrayBuffer> }): Promise<SubscriptionLike>;
};

export type PushDeps = {
  permission: () => NotificationPermission | string;
  requestPermission: () => Promise<NotificationPermission | string>;
  pushManager: () => Promise<PushManagerLike | null>;
  vapidKey: () => Promise<{ public_key?: string | null; reason?: string | null } | null>;
  save: (member: string, sub: { endpoint: string; keys: Record<string, string> }) => Promise<void>;
};

/** Base64url VAPID public key to the bytes `subscribe` wants. */
export function applicationServerKey(b64: string): Uint8Array<ArrayBuffer> {
  const pad = "=".repeat((4 - (b64.length % 4)) % 4);
  const raw = atob(b64.replace(/-/g, "+").replace(/_/g, "/") + pad);
  const out = new Uint8Array(new ArrayBuffer(raw.length));
  for (let i = 0; i < raw.length; i += 1) out[i] = raw.charCodeAt(i);
  return out;
}

export const PUSH_WORDS: Record<string, string> = {
  registered: "Notifications are on for this device.",
  no_member: "Sign in first.",
  unsupported: "This browser cannot do push notifications. On iPhone, add the console to your Home Screen first.",
  vapid_unavailable: "The server did not answer for its push key. Try again later.",
  vapid_unset: "This server has no push key set up, so it cannot send notifications.",
  permission_denied: "Notifications are blocked for this site. Allow them in the browser's site settings.",
  register_failed: "The server did not save this device. Try again.",
  off: "Notifications are off for this device.",
};

/** Ask permission, subscribe this browser, and store it for `member`. */
export async function enablePush(member: string | null, d: PushDeps): Promise<PushOutcome> {
  if (!member) return { registered: false, reason: "no_member" };
  const pm = await d.pushManager().catch(() => null);
  if (!pm) return { registered: false, reason: "unsupported" };
  const key = await d.vapidKey().catch(() => null);
  if (!key) return { registered: false, reason: "vapid_unavailable" };
  if (!key.public_key) return { registered: false, reason: key.reason || "vapid_unset" };
  let permission = d.permission();
  if (permission === "default") permission = await d.requestPermission();
  if (permission !== "granted") return { registered: false, reason: "permission_denied" };
  try {
    const sub = (await pm.getSubscription()) ?? (await pm.subscribe({ userVisibleOnly: true, applicationServerKey: applicationServerKey(key.public_key) }));
    const json = sub.toJSON();
    if (!json.endpoint || !json.keys) return { registered: false, reason: "register_failed" };
    await d.save(member, { endpoint: json.endpoint, keys: json.keys });
  } catch {
    return { registered: false, reason: "register_failed" };
  }
  return { registered: true, reason: "registered" };
}

/** Drop this browser's subscription. `forget` clears the server copy when it can. */
export async function disablePush(d: Pick<PushDeps, "pushManager">, forget: (endpoint: string) => Promise<void>): Promise<PushOutcome> {
  const pm = await d.pushManager().catch(() => null);
  const sub = pm ? await pm.getSubscription() : null;
  if (sub) {
    await forget(sub.endpoint).catch(() => undefined);
    await sub.unsubscribe();
  }
  return { registered: false, reason: "off" };
}

/** Why Turn on cannot run here, or null when it can. */
export function pushRefusal({ me, permission, hasKey }: { me: string | null; permission: string; hasKey: boolean | null }): string | null {
  if (!me) return "Sign in as a member to get notifications.";
  if (permission === "unsupported") return "This browser does not support notifications.";
  if (permission === "denied") return "Notifications are blocked for this site. Allow them in your browser's site settings (the icon next to the address), then reload.";
  if (hasKey === false) return "This server has no push key yet, so it cannot send notifications. An operator sets one up.";
  return null;
}
