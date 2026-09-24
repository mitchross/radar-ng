import { useEffect } from "react";
import { publishSharedState } from "../../modules/radar-shared-state";
import { sharedStateJson } from "../lib/sharedState";
import { useWeatherStore } from "../stores/useWeatherStore";

/** Mount once at the root: keeps the widget, CarPlay and Watch in step with the app. */
export function useSharedStatePublisher() {
  // A string, so the store's equality check skips unchanged states.
  const json = useWeatherStore((s) =>
    sharedStateJson({
      serverUrl: s.serverUrl,
      activePalette: s.activePalette,
      temperatureUnit: s.temperatureUnit,
      locationMode: s.locationMode,
      latitude: s.latitude,
      longitude: s.longitude,
      selectedPlace: s.selectedPlace,
      devicePlace: s.devicePlace,
      lastFixAt: s.lastFixAt,
    }),
  );
  useEffect(() => {
    publishSharedState(json);
  }, [json]);
}
