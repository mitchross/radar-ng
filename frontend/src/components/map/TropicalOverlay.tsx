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
        paint={{
          "fill-color": "#FF3B4A",
          "fill-opacity": 0.15,
          "fill-outline-color": "#FF3B4A",
        }}
      />
      {/* Forecast track line */}
      <Layer
        type="line"
        id="tropical-track"
        filter={["==", ["get", "kind"], "track"] as never}
        paint={{
          "line-color": "#FF3B4A",
          "line-width": 2,
          "line-dasharray": [2, 2] as never,
          "line-opacity": 0.9,
        }}
      />
      {/* Current storm position — circle + stroke */}
      <Layer
        type="circle"
        id="tropical-position"
        filter={["==", ["get", "kind"], "position"] as never}
        paint={{
          "circle-radius": 9,
          "circle-color": "#FF3B4A",
          "circle-stroke-color": "#FFFFFF",
          "circle-stroke-width": 2.5,
          "circle-opacity": 0.95,
        }}
      />
    </GeoJSONSource>
  );
}
