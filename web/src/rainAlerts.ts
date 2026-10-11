import {
  parseLastRainAlert,
  parseRainAlertLead,
  planRainAlert,
  shouldNotify,
  type RainAlertPlan,
} from "./shared/rainAlerts";
import type { RadarNowcastResponse } from "./shared/nowcastTypes";

/** Local browser alerts while the page is open; uses the app's planner. */
export class BrowserRainAlerts {
  enabled = this.read("enabled") === "1";
  leadMinutes = parseRainAlertLead(this.read("lead"));
  message = "Keep this page open for alerts. Use the app for background refresh.";
  private timer: ReturnType<typeof setTimeout> | undefined;
  private registration: ServiceWorkerRegistration | null = null;

  get supported(): boolean {
    return typeof Notification !== "undefined";
  }

  /** Chrome for Android and installed iOS Safari reject `new Notification`; they need a service worker registration. */
  async connect(): Promise<void> {
    if (this.registration || typeof navigator === "undefined" || !("serviceWorker" in navigator)) return;
    try {
      this.registration = await navigator.serviceWorker.register("/sw.js");
    } catch { /* insecure context or private mode: fall back to new Notification */ }
  }

  private read(key: string): string {
    try { return localStorage.getItem(`rng.rainAlerts.${key}`) ?? ""; } catch { return ""; }
  }

  private write(key: string, value: string): void {
    try { localStorage.setItem(`rng.rainAlerts.${key}`, value); } catch { /* private browsing */ }
  }

  async setEnabled(on: boolean): Promise<void> {
    this.cancel();
    let granted = false;
    if (on && this.supported) {
      await this.connect();
      try {
        granted = (Notification.permission === "granted" ? "granted" : await Notification.requestPermission()) === "granted";
      } catch { /* permission unavailable */ }
    }
    this.enabled = on && granted;
    this.write("enabled", this.enabled ? "1" : "0");
    this.message = on && !granted
      ? "Notifications are blocked or unavailable. Check browser permissions or use the app."
      : "Keep this page open for alerts. Use the app for background refresh.";
  }

  setLead(value: string): void {
    this.leadMinutes = parseRainAlertLead(value);
    this.write("lead", String(this.leadMinutes));
    this.cancel();
  }

  cancel(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
  }

  sync(nowcast: RadarNowcastResponse | null, place: string, now = Date.now()): void {
    this.cancel();
    if (!this.enabled || !this.supported || Notification.permission !== "granted") return;
    const plan = planRainAlert(nowcast, place, { leadMinutes: this.leadMinutes }, now);
    if (!plan || !shouldNotify(plan, parseLastRainAlert(this.read("last")), now)) return;
    this.timer = setTimeout(() => void this.deliver(plan), Math.max(0, plan.fireAt - now));
  }

  private async deliver(plan: RainAlertPlan): Promise<void> {
    this.timer = undefined;
    if (!this.enabled || !this.supported || Notification.permission !== "granted") return;
    const startInMin = Math.max(1, Math.round((plan.startAt - Date.now()) / 60_000));
    const body = plan.body.replace(/in about \d+ min/, `in about ${startInMin} min`);
    const options = { body, tag: "radar-ng-rain" };
    try {
      if (this.registration) await this.registration.showNotification(plan.title, options);
      else new Notification(plan.title, options);
      this.write("last", JSON.stringify({ key: plan.key, startAt: plan.startAt, firedAt: Date.now() }));
    } catch {
      this.message = "This browser cannot deliver alerts here. Use the app for rain alerts.";
    }
  }
}
