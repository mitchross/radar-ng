/**
 * Storm cell markers — dots at the centroid of each connected region ≥40 dBZ,
 * sized by area and coloured by peak intensity — plus, for history-backed
 * tracks, a 60-minute forecast path with 15-minute ticks and the past track
 * (same maths as the web site: lib/stormTracks).
 *
 * Always mounted; renders empty collections (and stops polling) while
 * `extrasVisible` is off so the map's native child count never churns.
 */
import { useMemo } from "react";
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useStormCells } from "../../hooks/useStormCells";
import { useBasemapStyle } from "../../hooks/useBasemapStyle";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useIsFocused } from "expo-router";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";
import { parseStormCells, stormTrackGeoJSON, type StormCellInfo } from "../../lib/stormTracks";

export function StormCellsOverlay({ onSelect }: { onSelect?: (cell: StormCellInfo) => void }) {
  const extrasVisible = useWeatherStore((s) => s.extrasVisible);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const { labelFont } = useBasemapStyle(serverUrl, mapStyle);
  // The Radar tab stays mounted behind other tabs; only poll while it's on screen.
  const focused = useIsFocused();
  const { data } = useStormCells(extrasVisible && focused);
  const geojson = extrasVisible && data ? (data as GeoJSON.FeatureCollection) : EMPTY_FEATURE_COLLECTION;
  const cells = useMemo(() => parseStormCells(extrasVisible ? data : null), [data, extrasVisible]);
  const tracks = useMemo(() => stormTrackGeoJSON(cells), [cells]);

  return (
    <>
    <GeoJSONSource id="storm-past-src" data={tracks.past}>
      <Layer type="line" id="storm-past" minzoom={6} paint={{ "line-color": "#6b7280", "line-width": 1.5, "line-dasharray": [1, 1.5] as never }} />
    </GeoJSONSource>
    <GeoJSONSource id="storm-tracks-src" data={tracks.tracks}>
      <Layer type="line" id="storm-track-casing" minzoom={5} layout={{ "line-cap": "round" }} paint={{ "line-color": "#111827", "line-width": 4.5, "line-opacity": 0.6 }} />
      <Layer type="line" id="storm-track" minzoom={5} layout={{ "line-cap": "round" }} paint={{ "line-color": ["case", ["get", "severe"], "#ff3b6b", "#ffffff"] as never, "line-width": 2 }} />
    </GeoJSONSource>
    <GeoJSONSource id="storm-ticks-src" data={tracks.ticks}>
      <Layer type="circle" id="storm-ticks" minzoom={6} paint={{ "circle-radius": 3, "circle-color": "#ffffff", "circle-stroke-color": "#111827", "circle-stroke-width": 1 }} />
      <Layer
        type="symbol"
        id="storm-tick-labels"
        minzoom={7}
        layout={{ "text-field": ["get", "label"] as never, "text-font": labelFont, "text-size": 10, "text-offset": [0, 1], "text-anchor": "top" }}
        paint={{ "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.2 }}
      />
    </GeoJSONSource>
    <GeoJSONSource
      id="storms-src"
      data={geojson}
      hitbox={{ top: 18, right: 18, bottom: 18, left: 18 }}
      onPress={(event) => {
        const id = Number(event.nativeEvent.features[0]?.properties?.cell_id);
        const cell = cells.find((c) => c.id === id);
        if (cell) {
          event.stopPropagation();
          onSelect?.(cell);
        }
      }}
    >
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
    </>
  );
}
