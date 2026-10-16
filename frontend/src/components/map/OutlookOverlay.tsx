/**
 * SPC day-1 severe thunderstorm outlook areas (/api/outlooks), in SPC's own category colors.
 * Mounted before the warnings so warnings draw on top. Empty while `extrasVisible` is off.
 */
import { useMemo } from "react";
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useQuery } from "@tanstack/react-query";
import { useIsFocused } from "expo-router";
import { fetchOutlook } from "../../lib/api";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";

export function OutlookOverlay() {
  const extrasVisible = useWeatherStore((s) => s.extrasVisible);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const focused = useIsFocused();
  const { data } = useQuery({
    queryKey: ["spc-outlook", serverUrl, 1],
    queryFn: ({ signal }) => fetchOutlook(serverUrl, 1, signal),
    enabled: extrasVisible && focused,
    refetchInterval: 10 * 60_000,
    staleTime: 10 * 60_000,
  });
  const geojson = useMemo(
    () => (extrasVisible && data?.features?.length ? data : EMPTY_FEATURE_COLLECTION),
    [data, extrasVisible],
  );

  return (
    <GeoJSONSource id="outlook-src" data={geojson}>
      <Layer type="fill" id="outlook-fill" paint={{ "fill-color": ["coalesce", ["get", "fill"], "#c1e9c1"] as never, "fill-opacity": 0.28 }} />
      <Layer type="line" id="outlook-line" paint={{ "line-color": ["coalesce", ["get", "stroke"], "#55bb55"] as never, "line-width": 1.5 }} />
    </GeoJSONSource>
  );
}
