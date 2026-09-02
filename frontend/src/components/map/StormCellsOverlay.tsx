/**
 * Storm cell markers — red/orange diamonds at the centroid of each
 * connected-component region ≥40dBZ in the live MRMS field. Size scales by
 * area, color by peak intensity.
 *
 * Always mounted; renders an empty collection (and stops polling) while
 * `extrasVisible` is off so the map's native child count never churns.
 */
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useStormCells } from "../../hooks/useStormCells";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";

export function StormCellsOverlay() {
  const extrasVisible = useWeatherStore((s) => s.extrasVisible);
  const { data } = useStormCells(extrasVisible);
  const geojson = extrasVisible && data ? (data as GeoJSON.FeatureCollection) : EMPTY_FEATURE_COLLECTION;

  return (
    <GeoJSONSource id="storms-src" data={geojson}>
      {/* Halo — soft glow sized by area_km2 */}
      <Layer
        type="circle"
        id="storms-halo"
        paint={{
          "circle-radius": [
            "interpolate",
            ["linear"],
            ["get", "area_km2"],
            25, 6,
            500, 18,
            5000, 36,
          ] as never,
          "circle-color": [
            "interpolate",
            ["linear"],
            ["get", "peak_dbz"],
            40, "#ff9f2e",
            50, "#ff4040",
            60, "#d02058",
            70, "#b24bff",
          ] as never,
          "circle-opacity": 0.18,
          "circle-blur": 0.6,
        }}
      />
      {/* Core dot */}
      <Layer
        type="circle"
        id="storms-core"
        paint={{
          "circle-radius": [
            "interpolate",
            ["linear"],
            ["get", "peak_dbz"],
            40, 4,
            60, 7,
            70, 9,
          ] as never,
          "circle-color": [
            "interpolate",
            ["linear"],
            ["get", "peak_dbz"],
            40, "#ff9f2e",
            50, "#ff4040",
            60, "#d02058",
            70, "#b24bff",
          ] as never,
          "circle-stroke-color": "#ffffff",
          "circle-stroke-width": 1.4,
          "circle-opacity": 0.95,
        }}
      />
    </GeoJSONSource>
  );
}
