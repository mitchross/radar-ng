/**
 * Lightning strikes overlay — renders the rolling 15-min buffer from the
 * self-hosted backend as yellow/white dots on the map. Fresh strikes (<60s)
 * pulse, older strikes fade toward transparent.
 *
 * Always mounted; renders an empty collection (and stops polling) while
 * `extrasVisible` is off so the map's native child count never churns.
 */
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useLightning } from "../../hooks/useLightning";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";

export function LightningOverlay() {
  const extrasVisible = useWeatherStore((s) => s.extrasVisible);
  const { data } = useLightning(extrasVisible);
  const geojson = extrasVisible && data ? (data as GeoJSON.FeatureCollection) : EMPTY_FEATURE_COLLECTION;

  return (
    <GeoJSONSource id="lightning-src" data={geojson}>
      {/* Halo (soft pulse) for fresh strikes */}
      <Layer
        type="circle"
        id="lightning-halo"
        filter={["<", ["get", "age_s"], 60] as never}
        paint={{
          "circle-radius": [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, 14,
            60, 6,
          ] as never,
          "circle-color": "#FFE066",
          "circle-opacity": [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, 0.55,
            60, 0.12,
          ] as never,
          "circle-blur": 0.6,
        }}
      />
      {/* Core strike dot — age-fade across the full 15-min buffer */}
      <Layer
        type="circle"
        id="lightning-dot"
        paint={{
          "circle-radius": [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, 5,
            900, 2,
          ] as never,
          "circle-color": [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, "#FFFFFF",
            30, "#FFE066",
            300, "#FFA94D",
            900, "#8B7CFF",
          ] as never,
          "circle-stroke-color": "#FFFFFF",
          "circle-stroke-width": 0.7,
          "circle-opacity": [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, 1,
            900, 0.25,
          ] as never,
        }}
      />
    </GeoJSONSource>
  );
}
