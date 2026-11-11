/**
 * Storm cell markers: dots at the centroid of each connected region ≥40 dBZ,
 * sized by area and coloured by peak intensity, plus, for history-backed
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
import type { PinnedPoint } from "../inspector/Eyedropper";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";
import { parseStormCells, stormTrackGeoJSON, type StormCellInfo } from "../../lib/stormTracks";
import { stormLayerOpacity, stormTrackTimeSec } from "../../../../web/src/shared/stormCone";

export function StormCellsOverlay({ onSelect }: { onSelect?: (cell: StormCellInfo, point?: PinnedPoint) => void }) {
  // Replay hides live storm overlays.
  const extrasVisible = useWeatherStore((s) => s.extrasVisible && s.source.kind === "live");
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const { labelFont } = useBasemapStyle(serverUrl, mapStyle);
  // The Radar tab stays mounted behind other tabs; only poll while it's on screen.
  const focused = useIsFocused();
  const { data } = useStormCells(extrasVisible && focused);
  const geojson = extrasVisible && data ? (data as GeoJSON.FeatureCollection) : EMPTY_FEATURE_COLLECTION;
  const cells = useMemo(() => parseStormCells(extrasVisible ? data : null), [data, extrasVisible]);
  const tracks = useMemo(() => stormTrackGeoJSON(cells), [cells]);
  // Fade the whole storm layer when the radar frame on screen is >10 min from the scan the tracks
  // were built from (scrubbing the loop); it returns at the live frame. Plain layer paint props on
  // the JS thread: no worklet or per-frame callback reads the shared module.
  const frameSec = useWeatherStore((s) => s.frames[s.currentFrameIndex]?.time ?? null);
  const fade = stormLayerOpacity(frameSec, stormTrackTimeSec(data));

  return (
    <>
    <GeoJSONSource id="storm-cones-src" data={tracks.cones}>
      <Layer type="fill" id="storm-cone-fill" minzoom={5} paint={{ "fill-color": ["case", ["get", "severe"], "#ff3b6b", "#ffffff"] as never, "fill-opacity": 0.16 * fade }} />
      <Layer type="line" id="storm-cone-line" minzoom={5} paint={{ "line-color": ["case", ["get", "severe"], "#ff3b6b", "#ffffff"] as never, "line-width": 1, "line-dasharray": [3, 2] as never, "line-opacity": 0.8 * fade }} />
    </GeoJSONSource>
    <GeoJSONSource id="storm-past-src" data={tracks.past}>
      <Layer type="line" id="storm-past" minzoom={6} paint={{ "line-color": "#6b7280", "line-width": 1.5, "line-dasharray": [1, 1.5] as never, "line-opacity": fade }} />
    </GeoJSONSource>
    <GeoJSONSource id="storm-tracks-src" data={tracks.tracks}>
      <Layer type="line" id="storm-track-casing" minzoom={5} layout={{ "line-cap": "round" }} paint={{ "line-color": "#111827", "line-width": 4.5, "line-opacity": 0.6 * fade }} />
      <Layer type="line" id="storm-track" minzoom={5} layout={{ "line-cap": "round" }} paint={{ "line-color": ["case", ["get", "severe"], "#ff3b6b", "#ffffff"] as never, "line-width": 2, "line-opacity": fade }} />
    </GeoJSONSource>
    <GeoJSONSource id="storm-ticks-src" data={tracks.ticks}>
      <Layer type="circle" id="storm-ticks" minzoom={6} paint={{ "circle-radius": 3, "circle-color": "#ffffff", "circle-stroke-color": "#111827", "circle-stroke-width": 1, "circle-opacity": fade, "circle-stroke-opacity": fade }} />
      <Layer
        type="symbol"
        id="storm-tick-labels"
        minzoom={7}
        layout={{ "text-field": ["get", "label"] as never, "text-font": labelFont, "text-size": 10, "text-offset": [0, 1], "text-anchor": "top" }}
        paint={{ "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.2, "text-opacity": fade }}
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
          const coordinates = event.nativeEvent.lngLat;
          if (coordinates) onSelect?.(cell, { lon: coordinates[0], lat: coordinates[1] });
          else onSelect?.(cell);
        }
      }}
    >
      {/* Halo: soft glow sized by area_km2 */}
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
          "circle-opacity": 0.18 * fade,
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
          "circle-opacity": 0.95 * fade,
          "circle-stroke-opacity": fade,
        }}
      />
    </GeoJSONSource>
    </>
  );
}
