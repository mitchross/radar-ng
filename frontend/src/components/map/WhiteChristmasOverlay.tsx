/**
 * White Christmas station dots, opened from the Home card (`/radar?wxmas=1`).
 * Always mounted; empty unless `visible`, so the map's child count never churns.
 */
import { useMemo } from "react";
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useWhiteChristmasStations } from "../../hooks/useWhiteChristmas";
import { whiteChristmasColorExpression } from "../../lib/whiteChristmas";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";

export function WhiteChristmasOverlay({ visible }: { visible: boolean }) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const { data } = useWhiteChristmasStations(serverUrl, visible);
  const geojson = useMemo(() => (visible && data?.features?.length ? data : EMPTY_FEATURE_COLLECTION), [data, visible]);
  return (
    <GeoJSONSource id="wxmas-src" data={geojson}>
      <Layer
        type="circle"
        id="wxmas-dots"
        paint={{
          "circle-radius": ["interpolate", ["linear"], ["zoom"], 3, 2.5, 8, 6] as never,
          "circle-color": whiteChristmasColorExpression() as never,
          "circle-stroke-color": "#111827",
          "circle-stroke-width": 1,
        }}
      />
      <Layer
        type="symbol"
        id="wxmas-labels"
        minzoom={6.5}
        layout={{
          "text-field": ["concat", ["to-string", ["round", ["get", "pct"]]], "%"] as never,
          "text-size": 11,
          "text-offset": [0, -1.2],
        }}
        paint={{ "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.4 }}
      />
    </GeoJSONSource>
  );
}
