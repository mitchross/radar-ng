/**
 * Forecast wall-time parsing shared by the web radar and the app. Open-Meteo
 * returns times at the forecast location with no offset ("2026-10-11T14:00"),
 * so reading them in the device zone would shift a chosen city in another
 * zone by the zone difference. Pure, so both clients share it.
 */

const WALL_TIME = /^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2}))?$/;

export interface ForecastDaily {
  time: readonly string[];
  sunrise: readonly (string | null)[];
  sunset: readonly (string | null)[];
}

/**
 * Open-Meteo wall time at the location → epoch ms. Without an offset (old
 * cached responses) it falls back to the device zone. NaN when unparseable.
 */
export function forecastTimeMs(value: string, utcOffsetSeconds?: number): number {
  const m = utcOffsetSeconds == null ? null : WALL_TIME.exec(value);
  if (!m || utcOffsetSeconds == null) return Date.parse(value);
  return Date.UTC(+m[1], +m[2] - 1, +m[3], +(m[4] ?? 0), +(m[5] ?? 0)) - utcOffsetSeconds * 1000;
}

/** The instant read as wall time at the location (the device zone without an offset). */
export function wallClock(atMs: number, utcOffsetSeconds?: number): { hour: number; dateKey: string } {
  const pad = (n: number) => String(n).padStart(2, "0");
  if (utcOffsetSeconds == null) {
    const at = new Date(atMs);
    return { hour: at.getHours(), dateKey: `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}` };
  }
  const w = new Date(atMs + utcOffsetSeconds * 1000);
  return { hour: w.getUTCHours(), dateKey: `${w.getUTCFullYear()}-${pad(w.getUTCMonth() + 1)}-${pad(w.getUTCDate())}` };
}

/** Sunrise/sunset (epoch ms) for the location's calendar day that contains `atMs`; NaN when unknown. */
export function sunTimesMs(daily: ForecastDaily, atMs: number, utcOffsetSeconds?: number): { sunrise: number; sunset: number } {
  const i = daily.time.indexOf(wallClock(atMs, utcOffsetSeconds).dateKey);
  const at = (list: readonly (string | null)[]) => {
    const v = i >= 0 ? list[i] : null;
    return v ? forecastTimeMs(v, utcOffsetSeconds) : NaN;
  };
  return { sunrise: at(daily.sunrise), sunset: at(daily.sunset) };
}

/**
 * Night for the instant's own day, not today's: outside that day's
 * sunrise–sunset, else a fixed 06:00–20:00 day when the sun times are missing.
 */
export function isNightAt(atMs: number, daily: ForecastDaily, utcOffsetSeconds?: number): boolean {
  const { sunrise, sunset } = sunTimesMs(daily, atMs, utcOffsetSeconds);
  if (Number.isFinite(sunrise) && Number.isFinite(sunset)) return atMs < sunrise || atMs > sunset;
  const { hour } = wallClock(atMs, utcOffsetSeconds);
  return hour < 6 || hour >= 20;
}
