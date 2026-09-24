import type { SelectedPlace } from "../types/location";

export function formatPlaceLabel(place: SelectedPlace): string {
  const parts = [place.name, place.admin1 || place.countryCode || place.country].filter(Boolean);
  return parts.join(", ");
}

/**
 * `fallback` is the city shown when device location is denied or unavailable;
 * pass it only in that case, so it is never labelled "My Location".
 */
export function activeLocationLabel(
  mode: "device" | "city",
  place: SelectedPlace | null,
  devicePlace: SelectedPlace | null,
  fallback: SelectedPlace | null = null,
): string {
  if (mode === "city" && place) return formatPlaceLabel(place);
  if (mode === "device" && fallback) return formatPlaceLabel(fallback);
  if (mode === "device" && devicePlace) return formatPlaceLabel(devicePlace);
  return "My Location";
}

export function activeLocationName(
  mode: "device" | "city",
  place: SelectedPlace | null,
  devicePlace: SelectedPlace | null,
  fallback: SelectedPlace | null = null,
): string {
  if (mode === "city" && place) return place.name;
  if (mode === "device" && fallback) return fallback.name;
  if (mode === "device" && devicePlace) return devicePlace.name;
  return "My Location";
}
