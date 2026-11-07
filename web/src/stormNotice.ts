import { approachCopy, parseSeenApproaches, stormApproaches, takeNewApproaches, type StormApproach, type StormCell } from "./shared/stormCone";

/**
 * "Storm reaches you in ~N min", on this device only. Opt-in (off by default).
 * The page shows the notice in the incoming-storm panel; a browser notification
 * is sent too when the user has allowed them (same permission as rain alerts),
 * once per storm cell id. Nothing leaves the page.
 */
export class BrowserStormNotice {
  enabled = this.read("enabled") === "1";
  private registration: ServiceWorkerRegistration | null = null;

  private read(key: string): string {
    try { return localStorage.getItem(`rng.stormNotice.${key}`) ?? ""; } catch { return ""; }
  }

  private write(key: string, value: string): void {
    try { localStorage.setItem(`rng.stormNotice.${key}`, value); } catch { /* private browsing */ }
  }

  async setEnabled(on: boolean): Promise<void> {
    this.enabled = on;
    this.write("enabled", on ? "1" : "0");
    if (!on || typeof Notification === "undefined") return;
    try {
      if (Notification.permission === "default") await Notification.requestPermission();
    } catch { /* the in-page notice still works without it */ }
  }

  /** Approaching cells for the place, and a browser notification for any not yet announced. */
  check(cells: StormCell[], place: [number, number], now = Date.now()): StormApproach[] {
    if (!this.enabled) return [];
    const approaches = stormApproaches(cells, place);
    const { fresh, seen } = takeNewApproaches(approaches, parseSeenApproaches(this.read("seen")), now);
    if (fresh.length > 0) {
      this.write("seen", JSON.stringify(seen));
      void this.deliver(fresh[0]);
    }
    return approaches;
  }

  private async deliver(a: StormApproach): Promise<void> {
    if (typeof Notification === "undefined" || Notification.permission !== "granted") return;
    const { title, body } = approachCopy(a);
    const options = { body, tag: `radar-ng-storm-${a.id}` };
    try {
      if (!this.registration && "serviceWorker" in navigator) this.registration = (await navigator.serviceWorker.getRegistration()) ?? null;
      if (this.registration) await this.registration.showNotification(title, options);
      else new Notification(title, options);
    } catch { /* the in-page notice already shows it */ }
  }
}
