import { useEffect } from "react";
import { AppState } from "react-native";
import * as Location from "expo-location";
import { useWeatherStore } from "../stores/useWeatherStore";
import { reverseGeocode } from "../lib/geocoding";
import { shouldRefreshFix } from "../lib/locationStatus";

// A fresh GPS fix can take a long time — or never arrive — indoors or on an
// emulator with no location set. Bound the wait so screens never get stuck.
const FRESH_FIX_TIMEOUT_MS = 10_000;

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return Promise.race([
    promise,
    new Promise<T>((_, reject) => {
      timer = setTimeout(() => reject(new Error("location-timeout")), ms);
    }),
  ]).finally(() => clearTimeout(timer));
}

let inFlight: Promise<void> | null = null;

/** One device-location pass: permission, cached fix, then a bounded fresh fix. */
function locateDevice(prompt: boolean): Promise<void> {
  if (inFlight) return inFlight;
  const store = useWeatherStore.getState;

  inFlight = (async () => {
    try {
      const { status } = prompt
        ? await Location.requestForegroundPermissionsAsync()
        : await Location.getForegroundPermissionsAsync();
      if (status !== "granted") {
        store().applyLocationFailure("denied");
        return;
      }

      // Fast path: the OS's cached fix renders something right away.
      try {
        const last = await Location.getLastKnownPositionAsync();
        if (last) store().applyDeviceFix(last.coords.latitude, last.coords.longitude, last.timestamp);
      } catch {
        // Fall through to a fresh request.
      }

      try {
        const loc = await withTimeout(
          Location.getCurrentPositionAsync({ accuracy: Location.Accuracy.Balanced }),
          FRESH_FIX_TIMEOUT_MS,
        );
        store().applyDeviceFix(loc.coords.latitude, loc.coords.longitude, loc.timestamp);
      } catch {
        store().applyLocationFailure("unavailable");
      }
    } catch {
      store().applyLocationFailure("unavailable");
    } finally {
      inFlight = null;
    }
  })();
  return inFlight;
}

/**
 * The single owner of the app's location. Mount once, at the root: it follows
 * the location mode, refreshes a stale device fix when the app returns to the
 * foreground, and resolves a place label for the device position.
 */
export function useLocationController() {
  const locationMode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const locationStatus = useWeatherStore((s) => s.locationStatus);
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  useEffect(() => {
    if (locationMode === "city") {
      if (selectedPlace) useWeatherStore.getState().setLocation(selectedPlace.latitude, selectedPlace.longitude);
      return;
    }
    void locateDevice(true);
    const subscription = AppState.addEventListener("change", (state) => {
      if (state !== "active") return;
      if (shouldRefreshFix(useWeatherStore.getState().lastFixAt)) void locateDevice(false);
    });
    return () => subscription.remove();
  }, [locationMode, selectedPlace]);

  // Label the device position. Coordinates only change when the device moves
  // to a new ~110 m cell, and reverseGeocode caches per cell besides.
  useEffect(() => {
    if (locationMode !== "device" || latitude === null || longitude === null) return;
    if (locationStatus === "denied" || locationStatus === "unavailable") return;
    let cancelled = false;
    reverseGeocode(serverUrl, latitude, longitude).then((place) => {
      if (!cancelled && place) useWeatherStore.getState().setDevicePlace(place);
    });
    return () => {
      cancelled = true;
    };
  }, [locationMode, latitude, longitude, locationStatus, serverUrl]);
}

/** Take a fresh device fix now (the Locate button), without prompting for permission again. */
export function refreshDeviceLocation(): void {
  if (useWeatherStore.getState().locationMode === "device") void locateDevice(false);
}
