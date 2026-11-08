import { BrowserRainAlerts } from "./rainAlerts";
import { alertLevel, parseAlertPreference, parseAlertSeen, polygonAlertBounds, selectAlertNotifications, TEST_ALERT, type AlertBounds, type AlertFeature, type AlertPreference } from "./shared/alertLevels";

/** NWS interruptions while this page is open; permission is shared with rain alerts. */
export class BrowserAlertNotifications {
  preference = parseAlertPreference(this.read("preference"));
  private seen = parseAlertSeen(this.read("seen"));
  constructor(private transport: BrowserRainAlerts, private openRadar: (bounds: AlertBounds | null) => void) {}

  private read(key: string): string {
    try { return localStorage.getItem(`rng.nwsAlerts.${key}`) ?? ""; } catch { return ""; }
  }
  private write(key: string, value: string): void {
    try { localStorage.setItem(`rng.nwsAlerts.${key}`, value); } catch { /* private mode */ }
  }
  async setPreference(value: AlertPreference): Promise<boolean> {
    this.preference = value;
    this.write("preference", value);
    return this.transport.requestPermission();
  }
  async test(): Promise<boolean> {
    if (!await this.transport.requestPermission()) return false;
    try {
      await this.transport.notify(TEST_ALERT.title, { body: TEST_ALERT.body, tag: "radar-ng-nws-test", data: { kind: "nws-alert", bounds: null } }, () => this.openRadar(null));
      return true;
    } catch { return false; }
  }
  async check(alerts: AlertFeature[]): Promise<void> {
    if (!this.transport.supported || Notification.permission !== "granted") return;
    const { fresh, seen } = selectAlertNotifications(alerts, { ...this.seen, ...parseAlertSeen(this.read("seen")) }, this.preference);
    this.seen = seen;
    this.write("seen", JSON.stringify(seen));
    for (const alert of fresh) {
      const bounds = polygonAlertBounds(alert.geometry);
      const level = alertLevel(alert.properties);
      try {
        await this.transport.notify(`${level.toUpperCase()} — ${alert.properties.event}`, {
          body: alert.properties.headline ?? alert.properties.event,
          tag: `radar-ng-nws-${alert.id ?? alert.properties.id}`,
          data: { kind: "nws-alert", bounds },
          // Browsers choose the system sound; the level is always visible in the title.
          silent: false,
        }, () => this.openRadar(bounds));
      } catch { /* remembered already: a blocked notification must not flood retries */ }
    }
  }
}
