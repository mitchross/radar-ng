/**
 * Lightning strikes overlay — renders the rolling 15-min buffer from the
 * self-hosted backend as yellow/white dots on the map. Fresh strikes (<60s)
 * pulse, older strikes fade toward transparent.
 */
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useLightning } from "../../hooks/useLightning";

export function LightningOverlay() {
  const { data } = useLightning();
  if (!data || data.features.length === 0) return null;

  return (
    <GeoJSONSource id="lightning-src" data={data as GeoJSON.FeatureCollection}>
      {/* Halo (soft pulse) for fresh strikes */}
      <Layer
        type="circle"
        id="lightning-halo"
        filter={["<", ["get", "age_s"], 60] as never}
        style={{
          circleRadius: [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, 14,
            60, 6,
          ] as never,
          circleColor: "#FFE066",
          circleOpacity: [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, 0.55,
            60, 0.12,
          ] as never,
          circleBlur: 0.6,
        }}
      />
      {/* Core strike dot — age-fade across the full 15-min buffer */}
      <Layer
        type="circle"
        id="lightning-dot"
        style={{
          circleRadius: [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, 5,
            900, 2,
          ] as never,
          circleColor: [
            "interpolate",
            ["linear"],
            ["get", "age_s"],
            0, "#FFFFFF",
            30, "#FFE066",
            300, "#FFA94D",
            900, "#8B7CFF",
          ] as never,
          circleStrokeColor: "#FFFFFF",
          circleStrokeWidth: 0.7,
          circleOpacity: [
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
