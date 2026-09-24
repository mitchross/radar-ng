import { DEFAULT_PLACE, useWeatherStore } from "../stores/useWeatherStore";
import { activeLocationLabel, activeLocationName } from "../lib/locationLabel";
import { isFallbackLocation, locationNotice } from "../lib/locationStatus";

/** What to call the location the weather is for, and when to qualify it. */
export function useActiveLocation() {
  const mode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const devicePlace = useWeatherStore((s) => s.devicePlace);
  const status = useWeatherStore((s) => s.locationStatus);
  const fallback = isFallbackLocation(mode, status) ? DEFAULT_PLACE : null;
  return {
    label: activeLocationLabel(mode, selectedPlace, devicePlace, fallback),
    name: activeLocationName(mode, selectedPlace, devicePlace, fallback),
    notice: locationNotice(mode, status, DEFAULT_PLACE.name),
    isFallback: fallback !== null,
    isDevice: mode === "device",
  };
}
