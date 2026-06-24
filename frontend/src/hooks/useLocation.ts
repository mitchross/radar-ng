import { useEffect, useState } from "react";
import * as Location from "expo-location";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";
import { reverseGeocode } from "../lib/geocoding";

// A fresh GPS fix can take a long time — or never arrive — indoors or on an
// emulator with no location set. Bound the wait so the Home screen never gets
// stuck on "Loading weather…" while latitude/longitude stay null.
const FRESH_FIX_TIMEOUT_MS = 10_000;

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return Promise.race([
    promise,
    new Promise<T>((_, reject) =>
      setTimeout(() => reject(new Error("location-timeout")), ms),
    ),
  ]);
}

export function useLocation() {
  const setLocation = useWeatherStore((s) => s.setLocation);
  const setDevicePlace = useWeatherStore((s) => s.setDevicePlace);
  const locationMode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    // Apply coords immediately so the forecast query can enable, then fill in
    // the city/state label as a best-effort follow-up.
    async function applyCoords(lat: number, lon: number) {
      if (cancelled) return;
      setLocation(lat, lon);
      try {
        const place = await reverseGeocode(lat, lon);
        if (!cancelled && place) setDevicePlace(place);
      } catch {
        // Label is best-effort; coords alone are enough to load weather.
      }
    }

    async function requestLocation() {
      if (locationMode === "city") {
        if (selectedPlace) setLocation(selectedPlace.latitude, selectedPlace.longitude);
        return;
      }

      const { status } = await Location.requestForegroundPermissionsAsync();
      if (status !== "granted") {
        setError("Location permission denied");
        await applyCoords(DEFAULTS.LATITUDE, DEFAULTS.LONGITUDE);
        return;
      }

      // Fast path: a cached fix renders the UI right away instead of blocking
      // on a fresh GPS lock.
      try {
        const last = await Location.getLastKnownPositionAsync();
        if (last && !cancelled) {
          await applyCoords(last.coords.latitude, last.coords.longitude);
        }
      } catch {
        // Ignore — fall through to a fresh request.
      }

      // Precise path: a fresh fix, but bounded so we never wait forever.
      try {
        const loc = await withTimeout(
          Location.getCurrentPositionAsync({ accuracy: Location.Accuracy.Balanced }),
          FRESH_FIX_TIMEOUT_MS,
        );
        await applyCoords(loc.coords.latitude, loc.coords.longitude);
      } catch {
        // Only fall back to a default if we still have nothing — otherwise the
        // last-known fix above already rendered a usable screen.
        if (!cancelled && useWeatherStore.getState().latitude === null) {
          setError("Location unavailable");
          await applyCoords(DEFAULTS.LATITUDE, DEFAULTS.LONGITUDE);
        }
      }
    }

    requestLocation();
    return () => {
      cancelled = true;
    };
  }, [locationMode, selectedPlace, setLocation, setDevicePlace]);

  return { error };
}
