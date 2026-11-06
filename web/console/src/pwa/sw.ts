// The console's service worker, built to /ui/sw.js. It keeps the static
// shell for offline starts (see policy.ts for what never gets cached) and
// shows Web Push notifications from the server's existing push sender.
import { SHELL_CACHE, SHELL_FILES, SHELL_SCOPE, cacheable, notificationTarget, pushPayload, shellKey, type PushPayload } from "./policy.ts";

type WaitEvent = Event & { waitUntil(p: Promise<unknown>): void };
type FetchEvt = WaitEvent & { request: Request; respondWith(r: Promise<Response>): void };
type PushEvt = WaitEvent & { data: { json(): unknown; text(): string } | null };
type ClickEvt = WaitEvent & { notification: { close(): void; data?: unknown } };
type WindowClientLike = { url: string; focus(): Promise<unknown>; navigate?(url: string): Promise<unknown> };
type Worker = {
  location: Location;
  addEventListener(type: string, fn: (e: never) => void): void;
  skipWaiting(): Promise<void>;
  registration: { showNotification(title: string, opts: NotificationOptions): Promise<void> };
  clients: { claim(): Promise<void>; matchAll(o: object): Promise<WindowClientLike[]>; openWindow(url: string): Promise<unknown> };
};

export function showPush(worker: Worker, event: PushEvt): Promise<void> {
  const d = event.data;
  const p = pushPayload(d ? () => d.json() : null, d ? () => d.text() : null);
  const options: NotificationOptions = { body: p.body, icon: `${SHELL_SCOPE}icon-192.png`, data: p };
  if (p.log_id) options.tag = `maidan-${p.log_id}`;
  return worker.registration.showNotification(p.title, options);
}

export async function openNotification(worker: Worker, event: ClickEvt): Promise<unknown> {
  event.notification.close();
  const target = new URL(notificationTarget(event.notification.data as Partial<PushPayload> | undefined), worker.location.origin).href;
  const scope = new URL(SHELL_SCOPE, worker.location.origin).href;
  const wins = await worker.clients.matchAll({ type: "window", includeUncontrolled: true });
  const open = wins.find((w) => w.url.startsWith(scope));
  if (!open) return worker.clients.openWindow(target);
  if (open.url !== target && typeof open.navigate === "function") await open.navigate(target).catch(() => {});
  return open.focus();
}

// `self` exists in the worker and not under node:test. typeof is safe either way.
if (typeof self !== "undefined") {
  const sw = self as unknown as Worker;
  sw.addEventListener("install", (e: WaitEvent) => {
    e.waitUntil(
      caches
        .open(SHELL_CACHE)
        .then((c) => c.addAll(SHELL_FILES.map((f) => `${SHELL_SCOPE}${f}`)))
        .then(() => sw.skipWaiting()),
    );
  });
  sw.addEventListener("activate", (e: WaitEvent) => {
    e.waitUntil(
      caches
        .keys()
        .then((keys) => Promise.all(keys.filter((k) => k.startsWith("maidan-console-shell-") && k !== SHELL_CACHE).map((k) => caches.delete(k))))
        .then(() => sw.clients.claim()),
    );
  });
  sw.addEventListener("fetch", (e: FetchEvt) => {
    const r = e.request;
    const key = shellKey({ method: r.method, url: r.url, origin: sw.location.origin, hasAuthorization: r.headers.has("Authorization") });
    if (!key) return; // Straight to the network, untouched.
    e.respondWith(
      fetch(r).then(
        (res) => {
          if (cacheable(res.status, res.type, res.headers.get("Cache-Control"))) {
            const copy = res.clone();
            e.waitUntil(caches.open(SHELL_CACHE).then((c) => c.put(key, copy)));
          }
          return res;
        },
        () => caches.match(key).then((hit) => hit ?? Response.error()),
      ),
    );
  });
  sw.addEventListener("push", (e: PushEvt) => {
    e.waitUntil(showPush(sw, e));
  });
  sw.addEventListener("notificationclick", (e: ClickEvt) => {
    e.waitUntil(openNotification(sw, e));
  });
}
