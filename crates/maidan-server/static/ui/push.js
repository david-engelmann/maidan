// @ts-check
import { api, apiReadPath, apiWritePath, headers } from "./api.js";

/**
 * Turn a base64url VAPID public key into the bytes subscribe expects.
 * @param {string} b64
 * @returns {Uint8Array}
 */
function applicationServerKey(b64) {
  const pad = "=".repeat((4 - (b64.length % 4)) % 4);
  const raw = atob(b64.replace(/-/g, "+").replace(/_/g, "/") + pad);
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i += 1) bytes[i] = raw.charCodeAt(i);
  return bytes;
}

/**
 * Register this browser for Web Push and store the subscription on the server.
 * Skips when the browser cannot, permission is denied, or VAPID is unset.
 * @param {string | null | undefined} memberId
 * @param {{ notification?: { permission: string, requestPermission: () => Promise<string> }, serviceWorker?: { register: (path: string) => Promise<{ pushManager: { getSubscription: () => Promise<any>, subscribe: (opts: any) => Promise<any> } }> } } | undefined} [overrides]
 * @returns {Promise<{ registered: boolean, reason: string }>}
 */
export async function registerBrowserPush(memberId, overrides) {
  if (!memberId) return { registered: false, reason: "no_member" };
  const notification = overrides?.notification ?? globalThis.Notification;
  const serviceWorker = overrides?.serviceWorker ?? globalThis.navigator?.serviceWorker;
  if (!notification || !serviceWorker || typeof serviceWorker.register !== "function") {
    return { registered: false, reason: "unsupported" };
  }
  let keyRes;
  try {
    keyRes = await api(apiReadPath("/web-push/vapid-public-key"), {
      headers: headers(),
      credentials: "include",
    });
  } catch (_e) {
    return { registered: false, reason: "vapid_unavailable" };
  }
  if (!keyRes.ok) return { registered: false, reason: "vapid_unavailable" };
  const keyBody = await keyRes.json();
  if (!keyBody.public_key) {
    return { registered: false, reason: keyBody.reason || "vapid_unset" };
  }
  let permission = notification.permission;
  if (permission === "default") {
    permission = await notification.requestPermission();
  }
  if (permission !== "granted") return { registered: false, reason: "permission_denied" };
  const registration = await serviceWorker.register(["", "ui", "static", "sw.js"].join("/"));
  const existing = await registration.pushManager.getSubscription();
  const subscription =
    existing ||
    (await registration.pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: applicationServerKey(keyBody.public_key),
    }));
  const json = subscription.toJSON();
  let res;
  try {
    res = await api(apiWritePath(`/members/${memberId}/push-subscriptions`), {
      method: "POST",
      headers: headers(true),
      credentials: "include",
      body: JSON.stringify({ endpoint: json.endpoint, keys: json.keys }),
    });
  } catch (_e) {
    return { registered: false, reason: "register_failed" };
  }
  if (!res.ok) return { registered: false, reason: "register_failed" };
  return { registered: true, reason: "registered" };
}
