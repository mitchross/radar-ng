import { useEffect, useState } from "react";
import * as Location from "expo-location";
import { useWeatherStore } from "../stores/useWeatherStore";
import { DEFAULTS } from "../lib/constants";

export function useLocation() {
  const setLocation = useWeatherStore((s) => s.setLocation);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function requestLocation() {
      const { status } = await Location.requestForegroundPermissionsAsync();
      if (status !== "granted") {
        setError("Location permission denied");
        setLocation(DEFAULTS.LATITUDE, DEFAULTS.LONGITUDE);
        return;
      }
      try {
        const loc = await Location.getCurrentPositionAsync({
          accuracy: Location.Accuracy.Balanced,
        });
        if (!cancelled) {
          setLocation(loc.coords.latitude, loc.coords.longitude);
        }
      } catch (e) {
        setError("Location unavailable");
        setLocation(DEFAULTS.LATITUDE, DEFAULTS.LONGITUDE);
      }
    }

    requestLocation();
    return () => {
      cancelled = true;
    };
  }, []);

  return { error };
}
