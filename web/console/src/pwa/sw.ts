// The console's service worker, built to /ui/sw.js. It keeps the static
// shell for offline starts (see policy.ts for what never gets cached) and
// shows Web Push notifications from the server's existing push sender.
import { NOTIFICATION_TARGET, SHELL_CACHE, SHELL_FILES, SHELL_SCOPE, cacheable, pushPayload, shellKey } from "./policy.ts";

type WaitEvent = Event & { waitUntil(p: Promise<unknown>): void };
type FetchEvt = WaitEvent & { request: Request; respondWith(r: Promise<Response>): void };
type PushEvt = WaitEvent & { data: { json(): unknown; text(): string } | null };
type ClickEvt = WaitEvent & { notification: { close(): void } };
type WindowClient = { url: string; focus(): Promise<unknown> };
type Worker = {
  location: Location;
  addEventListener(type: string, fn: (e: never) => void): void;
  skipWaiting(): Promise<void>;
  registration: { showNotification(title: string, opts: NotificationOptions): Promise<void> };
  clients: { claim(): Promise<void>; matchAll(o: object): Promise<WindowClient[]>; openWindow(url: string): Promise<unknown> };
};
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
  const d = e.data;
  const p = pushPayload(d ? () => d.json() : null, d ? () => d.text() : null);
  e.waitUntil(sw.registration.showNotification(p.title, { body: p.body, icon: `${SHELL_SCOPE}icon-192.png`, data: { log_id: p.log_id, kind: p.kind } }));
});

sw.addEventListener("notificationclick", (e: ClickEvt) => {
  e.notification.close();
  const target = new URL(NOTIFICATION_TARGET, sw.location.origin).href;
  const scope = new URL(SHELL_SCOPE, sw.location.origin).href;
  e.waitUntil(
    sw.clients.matchAll({ type: "window", includeUncontrolled: true }).then((wins) => {
      const open = wins.find((w) => w.url.startsWith(scope));
      return open ? open.focus() : sw.clients.openWindow(target);
    }),
  );
});
