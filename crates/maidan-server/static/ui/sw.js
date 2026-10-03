// Same object as `self`. Named so checkJs can treat this file as a service worker.
const sw = /** @type {ServiceWorkerGlobalScope} */ (/** @type {unknown} */ (self));
sw.addEventListener("push", (event) => {
  let payload = { title: "Maidan", body: "" };
  if (event.data) {
    try {
      const parsed = event.data.json();
      payload = {
        title: parsed.title || "Maidan",
        body: parsed.body || "",
        kind: parsed.kind,
        log_id: parsed.log_id,
      };
    } catch (_e) {
      payload.body = event.data.text();
    }
  }
  event.waitUntil(
    sw.registration.showNotification(payload.title || "Maidan", {
      body: payload.body || "",
      data: { log_id: payload.log_id, kind: payload.kind },
    }),
  );
});

sw.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL("/ui", sw.location.origin).href;
  event.waitUntil(
    sw.clients.matchAll({ type: "window", includeUncontrolled: true }).then((windows) => {
      for (const win of windows) {
        if (win.url.startsWith(target) && "focus" in win) return win.focus();
      }
      if (sw.clients.openWindow) return sw.clients.openWindow(target);
      return undefined;
    }),
  );
});
