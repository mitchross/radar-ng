// Minimal service worker: Chrome for Android and installed iOS Safari show
// notifications only through a registration. No fetch handler, nothing is cached.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((wins) => {
      const data = event.notification.data;
      const win = wins.find((w) => "focus" in w);
      if (win) {
        if (data?.kind === "nws-alert") win.postMessage(data);
        return win.focus();
      }
      const url = data?.kind === "nws-alert" && data.bounds
        ? `/?alertBounds=${encodeURIComponent(JSON.stringify(data.bounds))}` : "/";
      return self.clients.openWindow(url);
    }),
  );
});
