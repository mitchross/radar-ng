/**
 * Pure rules for where the device-location mode gets its coordinates and how
 * honestly the UI labels them. The app must never present a fallback city as
 * "My Location".
 */

/** Where device-mode coordinates currently come from. */
export type LocationStatus =
  | "locating" // no fix yet this session and nothing persisted
  | "live" // a fresh fix from this session
  | "last-known" // a persisted or OS-cached fix; possibly stale
  | "denied" // permission refused; showing the fallback city
  | "unavailable"; // no fix could be obtained; showing the fallback city

export interface DeviceFix {
  latitude: number;
  longitude: number;
  /** Epoch ms when the fix was taken. */
  at: number;
}

/** A fix older than this is refreshed when the app returns to the foreground. */
export const FIX_REFRESH_AFTER_MS = 10 * 60_000;
/** A fix younger than this counts as live rather than last-known. */
export const FIX_LIVE_MAX_AGE_MS = 2 * 60_000;

export function parseDeviceFix(value: string): DeviceFix | null {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value) as Partial<DeviceFix>;
    if (
      typeof parsed.latitude === "number" &&
      typeof parsed.longitude === "number" &&
      typeof parsed.at === "number" &&
      Math.abs(parsed.latitude) <= 90 &&
      Math.abs(parsed.longitude) <= 180 &&
      Number.isFinite(parsed.at)
    ) {
      return { latitude: parsed.latitude, longitude: parsed.longitude, at: parsed.at };
    }
  } catch {}
  return null;
}

export function statusForFix(fixAt: number, now = Date.now()): LocationStatus {
  return now - fixAt <= FIX_LIVE_MAX_AGE_MS ? "live" : "last-known";
}

export function shouldRefreshFix(lastFixAt: number | null, now = Date.now()): boolean {
  return lastFixAt === null || now - lastFixAt > FIX_REFRESH_AFTER_MS;
}

/** True when the shown coordinates are the fallback city, not the user's position. */
export function isFallbackLocation(mode: "device" | "city", status: LocationStatus): boolean {
  return mode === "device" && (status === "denied" || status === "unavailable");
}

/**
 * One line telling the user where the weather is for when that isn't their
 * live position, or null when nothing needs saying.
 */
export function locationNotice(
  mode: "device" | "city",
  status: LocationStatus,
  fallbackName: string,
): string | null {
  if (mode === "city") return null;
  switch (status) {
    case "denied":
      return `Location is off · showing ${fallbackName}`;
    case "unavailable":
      return `Location unavailable · showing ${fallbackName}`;
    case "last-known":
      return "Last known location";
    case "locating":
      return "Finding your location…";
    case "live":
      return null;
  }
}
