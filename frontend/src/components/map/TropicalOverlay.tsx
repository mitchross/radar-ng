/**
 * Tropical cyclone overlay — three layered styles keyed by `kind`:
 *   cone     → soft red translucent polygon (forecast uncertainty)
 *   track    → dashed red linestring (forecast positions)
 *   position → red pulsing symbol + storm name label
 */
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useTropical } from "../../hooks/useTropical";

export function TropicalOverlay() {
  const { data } = useTropical();
  if (!data || data.features.length === 0) return null;

  return (
    <GeoJSONSource id="tropical-src" data={data as GeoJSON.FeatureCollection}>
      {/* Cone (under everything else) */}
      <Layer
        type="fill"
        id="tropical-cone"
        filter={["==", ["get", "kind"], "cone"] as never}
        style={{
          fillColor: "#FF3B4A",
          fillOpacity: 0.15,
          fillOutlineColor: "#FF3B4A",
        }}
      />
      {/* Forecast track line */}
      <Layer
        type="line"
        id="tropical-track"
        filter={["==", ["get", "kind"], "track"] as never}
        style={{
          lineColor: "#FF3B4A",
          lineWidth: 2,
          lineDasharray: [2, 2] as never,
          lineOpacity: 0.9,
        }}
      />
      {/* Current storm position — circle + stroke */}
      <Layer
        type="circle"
        id="tropical-position"
        filter={["==", ["get", "kind"], "position"] as never}
        style={{
          circleRadius: 9,
          circleColor: "#FF3B4A",
          circleStrokeColor: "#FFFFFF",
          circleStrokeWidth: 2.5,
          circleOpacity: 0.95,
        }}
      />
    </GeoJSONSource>
  );
}
