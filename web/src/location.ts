/**
 * Ask before the browser's location prompt, and recover after a denial.
 * Mirrors the app's LocationPermissionCard (frontend/src/components/home/).
 */
export type LocationPermission = "granted" | "denied" | "prompt" | "unknown";
export type LocationCardKind = "undetermined" | "denied";

export const LOCATION_CARD_COPY: Record<LocationCardKind, { title: string; body: (city: string) => string; allow: boolean }> = {
  undetermined: {
    title: "Weather for where you are?",
    body: (city) => `Allow location and the forecast, radar and rain alerts follow you. Until then this is ${city}.`,
    allow: true,
  },
  denied: {
    title: "Location is off",
    body: (city) => `Showing ${city}. Turn location on in your browser's site settings, or pick a city to follow instead.`,
    allow: false,
  },
};

/** The geolocation permission without prompting; "unknown" where the Permissions API is missing or refuses. */
export async function queryLocationPermission(nav: Pick<Navigator, "permissions"> | undefined = globalThis.navigator): Promise<LocationPermission> {
  try {
    const status = await nav?.permissions?.query({ name: "geolocation" });
    return status?.state ?? "unknown";
  } catch {
    return "unknown";
  }
}

/** First visit: locate when already granted, ask first when unasked, stay quiet after a denial. */
export function bootLocationStep(permission: LocationPermission, hasStoredPlace: boolean): "locate" | "card" | "none" {
  if (hasStoredPlace || permission === "denied") return "none";
  return permission === "granted" ? "locate" : "card";
}

/** The locate button: straight to the fix when granted, otherwise the matching card. */
export function locateClickStep(permission: LocationPermission): "locate" | LocationCardKind {
  if (permission === "granted") return "locate";
  return permission === "denied" ? "denied" : "undetermined";
}

/** A refused prompt gets the recovery card; a timeout or no fix gets only a toast. */
export function cardAfterGeoError(code: number): LocationCardKind | null {
  return code === 1 ? "denied" : null;
}
