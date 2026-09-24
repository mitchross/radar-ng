/**
 * What the widget, CarPlay and Watch learn from the app. Serialized to one
 * JSON string so an unchanged state publishes nothing.
 */
import { formatPlaceLabel } from "./locationLabel";
import { roundCoords, PRECISION } from "./coordinates";
import type { LocationMode, SelectedPlace } from "../types/location";
import type { Palette, TemperatureUnit } from "../types/weather";

export const SHARED_STATE_VERSION = 1;

export interface SharedStateInput {
  serverUrl: string;
  activePalette: Palette;
  temperatureUnit: TemperatureUnit;
  locationMode: LocationMode;
  latitude: number | null;
  longitude: number | null;
  selectedPlace: SelectedPlace | null;
  devicePlace: SelectedPlace | null;
  lastFixAt: number | null;
}

export function sharedStateJson(s: SharedStateInput): string {
  const place = s.locationMode === "city" ? s.selectedPlace : s.devicePlace;
  const coords =
    s.latitude != null && s.longitude != null
      ? roundCoords(s.latitude, s.longitude, PRECISION.WEATHER)
      : null;
  return JSON.stringify({
    v: SHARED_STATE_VERSION,
    serverUrl: s.serverUrl,
    palette: s.activePalette,
    temperatureUnit: s.temperatureUnit,
    location: coords
      ? {
          // "city": a place the user chose, which the widget and Watch should show
          // instead of their own GPS. "device": the phone's last fix, a fallback.
          mode: s.locationMode,
          lat: coords.lat,
          lon: coords.lon,
          label: place ? formatPlaceLabel(place) : null,
          at: s.locationMode === "device" ? s.lastFixAt : null,
        }
      : null,
  });
}
