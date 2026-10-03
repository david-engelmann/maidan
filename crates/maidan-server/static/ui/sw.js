self.addEventListener("push", (event) => {
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
    self.registration.showNotification(payload.title || "Maidan", {
      body: payload.body || "",
      data: { log_id: payload.log_id, kind: payload.kind },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL("/ui", self.location.origin).href;
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((windows) => {
      for (const win of windows) {
        if (win.url.startsWith(target) && "focus" in win) return win.focus();
      }
      if (self.clients.openWindow) return self.clients.openWindow(target);
      return undefined;
    }),
  );
});
