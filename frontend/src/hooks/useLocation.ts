import { useEffect } from "react";
import { AppState } from "react-native";
import * as Location from "expo-location";
import { useWeatherStore } from "../stores/useWeatherStore";
import { reverseGeocode } from "../lib/geocoding";
import { shouldRefreshFix } from "../lib/locationStatus";

// A fresh GPS fix can take a long time, or never arrive, indoors or on an
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

interface InFlightPass {
  promise: Promise<void>;
  prompt: boolean;
  userInitiated: boolean;
  seq: number;
}

let inFlight: InFlightPass | null = null;
let passSeq = 0;

/**
 * One device-location pass: permission, cached fix, then a bounded fresh fix.
 * `prompt` raises the OS permission dialog; only the Home pre-prompt card's
 * "Allow" action does, so launch never surprises the user with it.
 * `userInitiated` (the Locate button) is the only case allowed to raise
 * Android's "turn on Location Accuracy" dialog; automatic launch and
 * foreground fixes must not nag with it every time.
 */
function locateDevice(prompt: boolean, userInitiated = false): Promise<void> {
  // Deduplicate only when the in-flight pass already satisfies this call's
  // escalation level; never let an automatic pass swallow an explicit user
  // permission prompt or Locate tap.
  if (inFlight && (!prompt || inFlight.prompt) && (!userInitiated || inFlight.userInitiated)) {
    return inFlight.promise;
  }
  const seq = ++passSeq;
  const isCurrent = () => passSeq === seq;
  const store = useWeatherStore.getState;

  const promise = (async () => {
    try {
      const { status, canAskAgain } = prompt
        ? await Location.requestForegroundPermissionsAsync()
        : await Location.getForegroundPermissionsAsync();
      if (!isCurrent()) return;
      if (status !== "granted") {
        store().applyLocationFailure(!prompt && canAskAgain ? "undetermined" : "denied");
        return;
      }

      // Fast path: the OS's cached fix renders something right away.
      try {
        const last = await Location.getLastKnownPositionAsync();
        if (!isCurrent()) return;
        if (last) store().applyDeviceFix(last.coords.latitude, last.coords.longitude, last.timestamp);
      } catch {
        // Fall through to a fresh request.
      }

      try {
        const loc = await withTimeout(
          Location.getCurrentPositionAsync({
            accuracy: Location.Accuracy.Balanced,
            mayShowUserSettingsDialog: userInitiated,
          }),
          FRESH_FIX_TIMEOUT_MS,
        );
        if (!isCurrent()) return;
        store().applyDeviceFix(loc.coords.latitude, loc.coords.longitude, loc.timestamp);
      } catch {
        if (!isCurrent()) return;
        store().applyLocationFailure("unavailable");
      }
    } catch {
      if (!isCurrent()) return;
      store().applyLocationFailure("unavailable");
    } finally {
      if (inFlight?.seq === seq) inFlight = null;
    }
  })();
  inFlight = { promise, prompt, userInitiated, seq };
  return promise;
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
    void locateDevice(false);
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
  if (useWeatherStore.getState().locationMode === "device") void locateDevice(false, true);
}

/** The pre-prompt card's "Allow": raise the OS permission dialog, then take a fix. */
export function requestDeviceLocation(): Promise<void> {
  return locateDevice(true, true);
}
