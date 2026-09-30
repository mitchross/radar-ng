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

  get supported(): boolean {
    return typeof Notification !== "undefined";
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
    this.timer = setTimeout(() => this.deliver(plan), Math.max(0, plan.fireAt - now));
  }

  private deliver(plan: RainAlertPlan): void {
    this.timer = undefined;
    if (!this.enabled || !this.supported || Notification.permission !== "granted") return;
    try {
      const startInMin = Math.max(1, Math.round((plan.startAt - Date.now()) / 60_000));
      const body = plan.body.replace(/in about \d+ min/, `in about ${startInMin} min`);
      new Notification(plan.title, { body, tag: "radar-ng-rain" });
      this.write("last", JSON.stringify({ key: plan.key, startAt: plan.startAt, firedAt: Date.now() }));
    } catch {
      // Some mobile browsers require a service worker even with permission.
      this.message = "This browser cannot deliver alerts here. Use the app for rain alerts.";
    }
  }
}
