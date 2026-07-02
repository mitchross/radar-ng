import type { SelectedPlace } from "../types/location";

export function formatPlaceLabel(place: SelectedPlace): string {
  const parts = [place.name, place.admin1 || place.countryCode || place.country].filter(Boolean);
  return parts.join(", ");
}

export function activeLocationLabel(
  mode: "device" | "city",
  place: SelectedPlace | null,
  devicePlace: SelectedPlace | null,
): string {
  if (mode === "city" && place) return formatPlaceLabel(place);
  if (mode === "device" && devicePlace) return formatPlaceLabel(devicePlace);
  return "My Location";
}

export function activeLocationName(
  mode: "device" | "city",
  place: SelectedPlace | null,
  devicePlace: SelectedPlace | null,
): string {
  if (mode === "city" && place) return place.name;
  if (mode === "device" && devicePlace) return devicePlace.name;
  return "My Location";
}
